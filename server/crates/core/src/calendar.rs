//! The Project work calendar: its definition rules, working-time arithmetic and the two commands
//! that read and write it.
//!
//! A calendar is created on purpose (there is no implicit default) and is one per Project. Times
//! are stored as UTC instants; the calendar's IANA zone only decides what "09:00 on a Monday"
//! means. PostgreSQL turns (local date, local time, zone) into instants, so the zone rules and
//! daylight saving come from the database that already resolves every other zone in the product.
//! A working minute is a minute of elapsed time between the instants of an interval's two ends,
//! so an interval is longer or shorter on a day the clocks change. All arithmetic after the
//! expansion is done in Rust over the list of instants.

use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, Utc, Weekday};
use penfii_contracts::{
    CalendarDefinition, CalendarInterval, CalendarSuggestion, CalendarWeek, ProjectCalendar,
    ProjectCalendarView,
};
use serde_json::json;
use sqlx::{PgConnection, PgPool, Row};
use std::collections::HashSet;
use uuid::Uuid;

use crate::error::CoreError;
use crate::project_role::{project_role, require_project_admin};

pub const DEFINITION_VERSION: i32 = 1;
const MAX_INTERVALS_PER_DAY: usize = 8;
const MAX_HOLIDAYS: usize = 1000;
const HOLIDAY_NAME_MAX: usize = 80;
/// How far from a Ticket's start a derived end is looked for.
pub const HORIZON_YEARS: u32 = 5;
/// Days expanded in one step while looking for the end of a duration.
const WINDOW_DAYS: i64 = 400;

/// One stretch of working time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkInterval {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

fn day_intervals(week: &CalendarWeek, day: Weekday) -> &[CalendarInterval] {
    match day {
        Weekday::Mon => &week.mon,
        Weekday::Tue => &week.tue,
        Weekday::Wed => &week.wed,
        Weekday::Thu => &week.thu,
        Weekday::Fri => &week.fri,
        Weekday::Sat => &week.sat,
        Weekday::Sun => &week.sun,
    }
}

/// Minutes since midnight of `HH:MM`. `24:00` is allowed only as the end of an interval.
fn clock_minutes(text: &str, is_end: bool) -> Result<u32, CoreError> {
    let bytes = text.as_bytes();
    let well_formed = bytes.len() == 5
        && bytes[2] == b':'
        && bytes[..2].iter().all(u8::is_ascii_digit)
        && bytes[3..].iter().all(u8::is_ascii_digit);
    if !well_formed {
        return Err(CoreError::validation("times are written HH:MM"));
    }
    let hours: u32 = text[..2].parse().unwrap_or(99);
    let minutes: u32 = text[3..].parse().unwrap_or(99);
    let total = hours * 60 + minutes;
    if minutes > 59 || total > 24 * 60 || (total == 24 * 60 && !is_end) {
        return Err(CoreError::validation(
            "a time of day must be 00:00 to 23:59",
        ));
    }
    Ok(total)
}

/// Checks a definition and returns it in its normal form (holidays sorted, names trimmed).
pub fn normalize(mut definition: CalendarDefinition) -> Result<CalendarDefinition, CoreError> {
    if definition.version != DEFINITION_VERSION {
        return Err(CoreError::validation(
            "unsupported calendar definition version",
        ));
    }
    let mut any = false;
    for day in [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ] {
        let intervals = day_intervals(&definition.week, day);
        if intervals.len() > MAX_INTERVALS_PER_DAY {
            return Err(CoreError::validation(format!(
                "a day has at most {MAX_INTERVALS_PER_DAY} working intervals"
            )));
        }
        let mut previous_end = 0;
        for interval in intervals {
            let from = clock_minutes(&interval.from, false)?;
            let to = clock_minutes(&interval.to, true)?;
            if to <= from {
                return Err(CoreError::validation(
                    "a working interval must end after it starts",
                ));
            }
            if from < previous_end {
                return Err(CoreError::validation(
                    "working intervals of a day must be in order and must not overlap",
                ));
            }
            previous_end = to;
            any = true;
        }
    }
    if !any {
        return Err(CoreError::validation(
            "a calendar needs working time on at least one weekday",
        ));
    }
    if definition.holidays.len() > MAX_HOLIDAYS {
        return Err(CoreError::validation(format!(
            "a calendar has at most {MAX_HOLIDAYS} holidays"
        )));
    }
    let earliest = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap_or_default();
    let latest = NaiveDate::from_ymd_opt(2100, 12, 31).unwrap_or_default();
    let mut seen = HashSet::new();
    for holiday in &mut definition.holidays {
        holiday.name = holiday.name.trim().to_owned();
        let length = holiday.name.chars().count();
        if length == 0 || length > HOLIDAY_NAME_MAX {
            return Err(CoreError::validation(format!(
                "a holiday name is 1..{HOLIDAY_NAME_MAX} characters"
            )));
        }
        if holiday.date < earliest || holiday.date > latest {
            return Err(CoreError::validation(
                "a holiday date must be between 2000 and 2100",
            ));
        }
        if !seen.insert(holiday.date) {
            return Err(CoreError::validation("a holiday date appears twice"));
        }
    }
    definition.holidays.sort_by_key(|holiday| holiday.date);
    Ok(definition)
}

/// What the screen proposes when a Project has no calendar: Monday to Friday, 09:00-18:00.
pub fn suggested_definition() -> CalendarDefinition {
    let day = || {
        vec![CalendarInterval {
            from: "09:00".into(),
            to: "18:00".into(),
        }]
    };
    CalendarDefinition {
        version: DEFINITION_VERSION,
        week: CalendarWeek {
            mon: day(),
            tue: day(),
            wed: day(),
            thu: day(),
            fri: day(),
            sat: Vec::new(),
            sun: Vec::new(),
        },
        holidays: Vec::new(),
    }
}

/// The local (date, from, to) rows of the working time in `[first, last)`, holidays left out.
fn local_rows(
    definition: &CalendarDefinition,
    first: NaiveDate,
    last: NaiveDate,
) -> (Vec<NaiveDate>, Vec<String>, Vec<String>) {
    let holidays: HashSet<NaiveDate> = definition.holidays.iter().map(|h| h.date).collect();
    let (mut dates, mut froms, mut tos) = (Vec::new(), Vec::new(), Vec::new());
    let mut day = first;
    while day < last {
        if !holidays.contains(&day) {
            for interval in day_intervals(&definition.week, day.weekday()) {
                dates.push(day);
                froms.push(interval.from.clone());
                tos.push(interval.to.clone());
            }
        }
        match day.succ_opt() {
            Some(next) => day = next,
            None => break,
        }
    }
    (dates, froms, tos)
}

/// The working intervals of `[first, last)` (local dates of the calendar's zone) as instants,
/// in time order. An interval that collapses to nothing because the clocks skipped it is dropped.
pub async fn expand(
    conn: &mut PgConnection,
    time_zone: &str,
    definition: &CalendarDefinition,
    first: NaiveDate,
    last: NaiveDate,
) -> Result<Vec<WorkInterval>, CoreError> {
    let (dates, froms, tos) = local_rows(definition, first, last);
    if dates.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT ((u.d + u.f::time) AT TIME ZONE $1) AS s, ((u.d + u.t::time) AT TIME ZONE $1) AS e
         FROM unnest($2::date[], $3::text[], $4::text[]) AS u(d, f, t)
         ORDER BY 1, 2",
    )
    .bind(time_zone)
    .bind(&dates)
    .bind(&froms)
    .bind(&tos)
    .fetch_all(&mut *conn)
    .await?;
    let mut intervals = Vec::with_capacity(rows.len());
    for row in rows {
        let interval = WorkInterval {
            start: row.try_get("s")?,
            end: row.try_get("e")?,
        };
        if interval.end > interval.start {
            intervals.push(interval);
        }
    }
    Ok(intervals)
}

/// The instant at which `minutes` of working time, counted from `start`, are done. Work begins at
/// `start` when that is working time and otherwise when the next interval opens, so zero minutes
/// is the first working instant at or after `start`. `None` when the intervals run out first.
pub fn add_working_minutes(
    intervals: &[WorkInterval],
    start: DateTime<Utc>,
    minutes: i64,
) -> Option<DateTime<Utc>> {
    if minutes < 0 {
        return None;
    }
    let mut remaining = Duration::minutes(minutes);
    let mut index = intervals.partition_point(|interval| interval.end <= start);
    let mut position = start;
    while let Some(interval) = intervals.get(index) {
        position = position.max(interval.start);
        let available = interval.end - position;
        if remaining <= available {
            return Some(position + remaining);
        }
        remaining -= available;
        index += 1;
    }
    None
}

/// Working time between two instants (tests only; the product derives ends, not durations).
#[cfg(test)]
pub fn working_minutes_between(
    intervals: &[WorkInterval],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> i64 {
    intervals
        .iter()
        .map(|interval| {
            let start = interval.start.max(from);
            let end = interval.end.min(to);
            (end - start).num_seconds().max(0)
        })
        .sum::<i64>()
        / 60
}

pub(crate) struct StoredCalendar {
    pub version: i64,
    pub time_zone: String,
    pub definition: CalendarDefinition,
    pub updated_at: DateTime<Utc>,
}

pub(crate) async fn load(
    conn: &mut PgConnection,
    project_id: Uuid,
) -> Result<Option<StoredCalendar>, CoreError> {
    let row = sqlx::query(
        "SELECT version, time_zone, definition, updated_at FROM penfii.project_calendars WHERE project_id=$1",
    )
    .bind(project_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(|row| {
        Ok(StoredCalendar {
            version: row.try_get("version")?,
            time_zone: row.try_get("time_zone")?,
            updated_at: row.try_get("updated_at")?,
            definition: serde_json::from_value(row.try_get("definition")?)
                .map_err(|e| CoreError::Internal(format!("stored calendar: {e}")))?,
        })
    })
    .transpose()
}

/// The planned end of `minutes` of work from `start` under the Project's current calendar, or
/// `None` when the Project has no calendar or the end lies beyond the five-year horizon.
pub async fn planned_end(
    conn: &mut PgConnection,
    project_id: Uuid,
    start: DateTime<Utc>,
    minutes: i64,
) -> Result<Option<DateTime<Utc>>, CoreError> {
    let Some(calendar) = load(conn, project_id).await? else {
        return Ok(None);
    };
    let local_start: NaiveDate =
        sqlx::query_scalar("SELECT ($1::timestamptz AT TIME ZONE $2)::date")
            .bind(start)
            .bind(&calendar.time_zone)
            .fetch_one(&mut *conn)
            .await?;
    let Some(horizon) = local_start.checked_add_months(Months::new(HORIZON_YEARS * 12)) else {
        return Ok(None);
    };
    let mut intervals: Vec<WorkInterval> = Vec::new();
    let mut from = local_start;
    while from < horizon {
        let to = (from + Duration::days(WINDOW_DAYS)).min(horizon);
        intervals.extend(expand(conn, &calendar.time_zone, &calendar.definition, from, to).await?);
        if let Some(end) = add_working_minutes(&intervals, start, minutes) {
            return Ok(Some(end));
        }
        from = to;
    }
    Ok(None)
}

/// The planned end of many schedules at once: `(start, minutes)` in, the end or `None` out, in the
/// same order. Intervals are expanded in windows as the starts move forward, so a list of schedules
/// costs a few queries and not one per schedule.
pub async fn planned_ends(
    conn: &mut PgConnection,
    project_id: Uuid,
    items: &[(DateTime<Utc>, i64)],
) -> Result<Vec<Option<DateTime<Utc>>>, CoreError> {
    let mut ends = vec![None; items.len()];
    if items.is_empty() {
        return Ok(ends);
    }
    let Some(calendar) = load(conn, project_id).await? else {
        return Ok(ends);
    };
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|i| items[*i].0);
    let mut intervals: Vec<WorkInterval> = Vec::new();
    let mut covered_to: Option<NaiveDate> = None;
    for index in order {
        let (start, minutes) = items[index];
        let local: NaiveDate = sqlx::query_scalar("SELECT ($1::timestamptz AT TIME ZONE $2)::date")
            .bind(start)
            .bind(&calendar.time_zone)
            .fetch_one(&mut *conn)
            .await?;
        let Some(horizon) = local.checked_add_months(Months::new(HORIZON_YEARS * 12)) else {
            continue;
        };
        // A schedule that starts after what is expanded starts a new stretch; the gap holds nothing it needs.
        if covered_to.is_none_or(|covered| local >= covered) {
            intervals.clear();
            covered_to = Some(local);
        }
        loop {
            if let Some(end) = add_working_minutes(&intervals, start, minutes) {
                ends[index] = Some(end);
                break;
            }
            let from = covered_to.unwrap_or(local);
            if from >= horizon {
                break;
            }
            let to = (from + Duration::days(WINDOW_DAYS)).min(horizon);
            intervals
                .extend(expand(conn, &calendar.time_zone, &calendar.definition, from, to).await?);
            covered_to = Some(to);
        }
    }
    Ok(ends)
}

pub async fn get_calendar(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
) -> Result<ProjectCalendarView, CoreError> {
    let mut tx = pool.begin().await?;
    let (workspace, role) = project_role(&mut tx, project_id, actor_id).await?;
    // A Project's inside is for its participants; every other member is told it does not exist.
    let role = role.ok_or(CoreError::NotFound)?;
    let can_manage = crate::project_role::can_manage(&role);
    let stored = load(&mut tx, project_id).await?;
    let view = match stored {
        Some(calendar) => ProjectCalendarView {
            calendar: Some(ProjectCalendar {
                project_id,
                version: calendar.version,
                time_zone: calendar.time_zone,
                definition: calendar.definition,
                updated_at: calendar.updated_at,
            }),
            suggestion: None,
            can_manage,
        },
        None => {
            let time_zone: String =
                sqlx::query_scalar("SELECT time_zone FROM penfii.workspaces WHERE id=$1")
                    .bind(workspace)
                    .fetch_one(&mut *tx)
                    .await?;
            ProjectCalendarView {
                calendar: None,
                suggestion: Some(CalendarSuggestion {
                    time_zone,
                    definition: suggested_definition(),
                }),
                can_manage,
            }
        }
    };
    tx.commit().await?;
    Ok(view)
}

fn version_conflict(current: i64) -> CoreError {
    CoreError::conflict_extra(
        "calendar_version_conflict",
        "calendar version conflict",
        serde_json::from_value(json!({"currentVersion": current})).unwrap_or_default(),
    )
}

/// Creates the calendar (`expected_version` 0) or replaces it (`expected_version` is the version
/// the person read). Only a Project admin may; a stale version never overwrites.
pub async fn put_calendar(
    pool: &PgPool,
    actor_id: Uuid,
    project_id: Uuid,
    expected_version: i64,
    time_zone: &str,
    definition: CalendarDefinition,
) -> Result<ProjectCalendar, CoreError> {
    let definition = normalize(definition)?;
    let time_zone = time_zone.trim();
    let mut tx = pool.begin().await?;
    require_project_admin(&mut tx, project_id, actor_id).await?;
    let known: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name=$1)")
            .bind(time_zone)
            .fetch_one(&mut *tx)
            .await?;
    if !known {
        return Err(CoreError::validation("unknown time zone"));
    }
    let body = serde_json::to_value(&definition).map_err(|e| CoreError::Internal(e.to_string()))?;
    let row = if expected_version == 0 {
        sqlx::query(
            "INSERT INTO penfii.project_calendars(project_id, time_zone, definition, updated_by)
             VALUES($1,$2,$3,$4) ON CONFLICT (project_id) DO NOTHING
             RETURNING version, updated_at",
        )
        .bind(project_id)
        .bind(time_zone)
        .bind(&body)
        .bind(actor_id)
        .fetch_optional(&mut *tx)
        .await?
    } else {
        sqlx::query(
            "UPDATE penfii.project_calendars
             SET time_zone=$3, definition=$4, version=version+1, updated_by=$5, updated_at=now()
             WHERE project_id=$1 AND version=$2 RETURNING version, updated_at",
        )
        .bind(project_id)
        .bind(expected_version)
        .bind(time_zone)
        .bind(&body)
        .bind(actor_id)
        .fetch_optional(&mut *tx)
        .await?
    };
    let Some(row) = row else {
        let current: Option<i64> =
            sqlx::query_scalar("SELECT version FROM penfii.project_calendars WHERE project_id=$1")
                .bind(project_id)
                .fetch_optional(&mut *tx)
                .await?;
        return Err(version_conflict(current.unwrap_or(0)));
    };
    tx.commit().await?;
    Ok(ProjectCalendar {
        project_id,
        version: row.try_get("version")?,
        time_zone: time_zone.to_owned(),
        definition,
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use penfii_contracts::CalendarHoliday;

    fn day(from: &str, to: &str) -> Vec<CalendarInterval> {
        vec![CalendarInterval {
            from: from.into(),
            to: to.into(),
        }]
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn interval(start: &str, end: &str) -> WorkInterval {
        WorkInterval {
            start: at(start),
            end: at(end),
        }
    }

    #[test]
    fn the_suggested_calendar_is_valid() {
        assert!(normalize(suggested_definition()).is_ok());
    }

    #[test]
    fn a_definition_must_have_working_time_ordered_intervals_and_unique_holidays() {
        let mut empty = suggested_definition();
        empty.week = CalendarWeek::default();
        assert!(normalize(empty).is_err());

        let mut backwards = suggested_definition();
        backwards.week.mon = day("18:00", "09:00");
        assert!(normalize(backwards).is_err());

        let mut overlapping = suggested_definition();
        overlapping.week.mon = vec![
            CalendarInterval {
                from: "09:00".into(),
                to: "13:00".into(),
            },
            CalendarInterval {
                from: "12:00".into(),
                to: "18:00".into(),
            },
        ];
        assert!(normalize(overlapping).is_err());

        let mut midnight = suggested_definition();
        midnight.week.sat = day("20:00", "24:00");
        assert!(normalize(midnight).is_ok());
        let mut start_at_24 = suggested_definition();
        start_at_24.week.sat = day("24:00", "24:00");
        assert!(normalize(start_at_24).is_err());

        let mut sloppy = suggested_definition();
        sloppy.week.mon = day("9:00", "18:00");
        assert!(normalize(sloppy).is_err());

        let date = NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
        let mut twice = suggested_definition();
        twice.holidays = vec![
            CalendarHoliday {
                date,
                name: "Christmas".into(),
            },
            CalendarHoliday {
                date,
                name: "Again".into(),
            },
        ];
        assert!(normalize(twice).is_err());

        let mut wrong_version = suggested_definition();
        wrong_version.version = 2;
        assert!(normalize(wrong_version).is_err());
    }

    #[test]
    fn holidays_are_sorted_and_names_trimmed() {
        let mut definition = suggested_definition();
        definition.holidays = vec![
            CalendarHoliday {
                date: NaiveDate::from_ymd_opt(2026, 12, 25).unwrap(),
                name: " Christmas ".into(),
            },
            CalendarHoliday {
                date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                name: "New year".into(),
            },
        ];
        let normal = normalize(definition).unwrap();
        assert_eq!(normal.holidays[0].name, "New year");
        assert_eq!(normal.holidays[1].name, "Christmas");
    }

    #[test]
    fn zero_minutes_is_the_first_working_instant_and_work_spans_intervals() {
        let intervals = [
            interval("2026-11-02T09:00:00Z", "2026-11-02T12:00:00Z"),
            interval("2026-11-02T13:00:00Z", "2026-11-02T18:00:00Z"),
            interval("2026-11-03T09:00:00Z", "2026-11-03T18:00:00Z"),
        ];
        // Before opening, inside, in the lunch gap, at a closing instant.
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-02T07:00:00Z"), 0),
            Some(at("2026-11-02T09:00:00Z"))
        );
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-02T10:30:00Z"), 0),
            Some(at("2026-11-02T10:30:00Z"))
        );
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-02T12:30:00Z"), 0),
            Some(at("2026-11-02T13:00:00Z"))
        );
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-02T12:00:00Z"), 0),
            Some(at("2026-11-02T13:00:00Z"))
        );
        // 4 hours from 10:00: two hours to noon, then two after lunch.
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-02T10:00:00Z"), 240),
            Some(at("2026-11-02T15:00:00Z"))
        );
        // Ending exactly when an interval closes stays at that instant.
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-02T09:00:00Z"), 180),
            Some(at("2026-11-02T12:00:00Z"))
        );
        // Over the end of the day continues the next morning; beyond the list there is no answer.
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-02T16:00:00Z"), 180),
            Some(at("2026-11-03T10:00:00Z"))
        );
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-03T17:00:00Z"), 61),
            None
        );
        assert_eq!(
            add_working_minutes(&intervals, at("2026-11-02T09:00:00Z"), -1),
            None
        );
    }

    #[test]
    fn adding_and_measuring_working_time_agree() {
        let intervals = [
            interval("2026-11-02T09:00:00Z", "2026-11-02T12:00:00Z"),
            interval("2026-11-02T13:00:00Z", "2026-11-02T18:00:00Z"),
            interval("2026-11-03T09:00:00Z", "2026-11-03T18:00:00Z"),
        ];
        let start = at("2026-11-02T10:15:00Z");
        for minutes in [1, 45, 105, 106, 200, 465, 600] {
            let end = add_working_minutes(&intervals, start, minutes).unwrap();
            assert_eq!(
                working_minutes_between(&intervals, start, end),
                minutes,
                "{minutes}"
            );
        }
    }
}
