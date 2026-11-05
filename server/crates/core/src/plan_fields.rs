//! The planning values of a Ticket: a Task's estimate and its planned schedule.
//!
//! The estimate is a decimal with at most two decimals, kept as text outside the database so a sum
//! never goes through a float. Only a Task carries one, and only in the Project's unit; nothing is
//! ever converted between units. A schedule is a planned start plus durations in working minutes;
//! its end is derived from the Project calendar when read and is never stored.

use chrono::{DateTime, Datelike, Utc};
use penfii_contracts::{TicketEstimate, TicketSchedule};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::CoreError;

/// Longest estimate integer part: the column is `numeric(10,2)`.
const ESTIMATE_INTEGER_DIGITS: usize = 8;
/// Ten years of continuous minutes: far past any plan, short of overflow.
pub const MAX_MINUTES: i64 = 5_256_000;

/// An estimate as sent. `unit`, when given, must be the Project's unit.
#[derive(Debug, Clone)]
pub struct EstimateInput {
    pub value: String,
    pub unit: Option<String>,
}

/// A schedule as sent.
#[derive(Debug, Clone)]
pub struct ScheduleInput {
    pub planned_start_at: Option<DateTime<Utc>>,
    pub duration_minutes: Option<i64>,
    pub is_schedule_milestone: bool,
}

/// `"5"`, `"0.5"`, `"12.25"` as hundredths. No sign, no exponent, no third decimal.
pub fn parse_hundredths(text: &str) -> Result<i64, CoreError> {
    let text = text.trim();
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty()
        || whole.len() > ESTIMATE_INTEGER_DIGITS
        || fraction.len() > 2
        || !digits(whole)
        || !digits(fraction)
        || (text.contains('.') && fraction.is_empty())
    {
        return Err(CoreError::validation(
            "an estimate is a number with at most two decimals",
        ));
    }
    let whole: i64 = whole.parse().unwrap_or(0);
    let fraction: i64 = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().unwrap_or(0) * 10,
        _ => fraction.parse().unwrap_or(0),
    };
    Ok(whole * 100 + fraction)
}

/// Hundredths as the shortest decimal text: `500` is `"5"`, `50` is `"0.5"`.
pub fn format_hundredths(value: i64) -> String {
    let (whole, fraction) = (value / 100, value % 100);
    match fraction {
        0 => whole.to_string(),
        f if f % 10 == 0 => format!("{whole}.{}", f / 10),
        f => format!("{whole}.{f:02}"),
    }
}

/// The text PostgreSQL gives for a `numeric(10,2)` (`"5.00"`) in the same shortest form.
pub fn normalize_decimal_text(text: &str) -> String {
    match parse_hundredths(text) {
        Ok(value) => format_hundredths(value),
        Err(_) => text.to_owned(),
    }
}

/// Checks an estimate input against the Ticket and the Project and returns what to store.
/// `None` clears it. A Project whose unit is `NONE` takes no estimate.
pub async fn resolve_estimate(
    conn: &mut PgConnection,
    project_id: Uuid,
    hierarchy_kind: &str,
    input: Option<EstimateInput>,
) -> Result<Option<TicketEstimate>, CoreError> {
    let Some(input) = input else {
        return Ok(None);
    };
    if hierarchy_kind != "TASK" {
        return Err(CoreError::rule(
            "estimate_not_allowed",
            "only a Task carries an estimate",
        ));
    }
    let unit: String = sqlx::query_scalar("SELECT estimate_unit FROM penfii.projects WHERE id=$1")
        .bind(project_id)
        .fetch_one(&mut *conn)
        .await?;
    if unit == "NONE" {
        return Err(CoreError::rule(
            "estimate_not_allowed",
            "this Project does not estimate; choose a unit in its settings first",
        ));
    }
    if input.unit.as_deref().is_some_and(|given| given != unit) {
        return Err(CoreError::rule(
            "estimate_unit_mismatch",
            "the estimate is not in this Project's unit",
        ));
    }
    let value = format_hundredths(parse_hundredths(&input.value)?);
    Ok(Some(TicketEstimate { value, unit }))
}

/// Checks a schedule input and returns what to store, or `None` when it says nothing.
pub fn resolve_schedule(input: Option<ScheduleInput>) -> Result<Option<TicketSchedule>, CoreError> {
    let Some(input) = input else {
        return Ok(None);
    };
    let invalid = |message: &str| Err(CoreError::rule("schedule_invalid", message.to_owned()));
    if let Some(minutes) = input.duration_minutes {
        if minutes < 0 {
            return invalid("a duration cannot be negative");
        }
        if minutes > MAX_MINUTES {
            return invalid("a duration is limited to ten years of working minutes");
        }
        if minutes == 0 && !input.is_schedule_milestone {
            return invalid("only a schedule milestone may have a duration of 0");
        }
    }
    if input.is_schedule_milestone && input.duration_minutes.is_some_and(|m| m > 0) {
        return invalid("a schedule milestone has no duration");
    }
    if let Some(start) = input.planned_start_at
        && !(2000..=2100).contains(&start.year())
    {
        return invalid("the planned start must be between 2000 and 2100");
    }
    if input.planned_start_at.is_none()
        && input.duration_minutes.is_none()
        && !input.is_schedule_milestone
    {
        return Ok(None);
    }
    Ok(Some(TicketSchedule {
        planned_start_at: input.planned_start_at,
        duration_minutes: input.duration_minutes,
        is_schedule_milestone: input.is_schedule_milestone,
        planned_end_at: None,
    }))
}

/// Stores a Ticket's schedule, or removes it. The row exists only while a schedule is set.
pub async fn write_schedule(
    conn: &mut PgConnection,
    ticket_id: Uuid,
    schedule: Option<&TicketSchedule>,
) -> Result<(), CoreError> {
    match schedule {
        Some(schedule) => {
            sqlx::query(
                "INSERT INTO penfii.ticket_schedules(ticket_id,planned_start_at,duration_minutes,is_milestone)
                 VALUES($1,$2,$3,$4)
                 ON CONFLICT (ticket_id) DO UPDATE SET planned_start_at=EXCLUDED.planned_start_at,
                   duration_minutes=EXCLUDED.duration_minutes,
                   is_milestone=EXCLUDED.is_milestone",
            )
            .bind(ticket_id)
            .bind(schedule.planned_start_at)
            .bind(schedule.duration_minutes)
            .bind(schedule.is_schedule_milestone)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM penfii.ticket_schedules WHERE ticket_id=$1")
                .bind(ticket_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// The derived end of a stored schedule under the Project's current calendar.
pub async fn with_planned_end(
    conn: &mut PgConnection,
    project_id: Uuid,
    schedule: &mut TicketSchedule,
) -> Result<(), CoreError> {
    schedule.planned_end_at = None;
    let (Some(start), Some(minutes)) = (
        schedule.planned_start_at,
        schedule
            .duration_minutes
            .or(if schedule.is_schedule_milestone {
                Some(0)
            } else {
                None
            }),
    ) else {
        return Ok(());
    };
    schedule.planned_end_at =
        crate::calendar::planned_end(conn, project_id, start, minutes).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_round_trip_in_their_shortest_form() {
        for (text, hundredths, shown) in [
            ("5", 500, "5"),
            ("0", 0, "0"),
            ("0.5", 50, "0.5"),
            ("0.50", 50, "0.5"),
            ("12.25", 1225, "12.25"),
            ("007.10", 710, "7.1"),
            ("99999999.99", 9_999_999_999, "99999999.99"),
        ] {
            assert_eq!(parse_hundredths(text).unwrap(), hundredths, "{text}");
            assert_eq!(format_hundredths(hundredths), shown, "{text}");
        }
        assert_eq!(normalize_decimal_text("5.00"), "5");
        assert_eq!(normalize_decimal_text("0.50"), "0.5");
    }

    #[test]
    fn malformed_estimates_are_refused() {
        for text in [
            "",
            " ",
            "-1",
            "+1",
            "1e3",
            "1.234",
            ".5",
            "5.",
            "1,5",
            "abc",
            "123456789",
            "1 2",
        ] {
            assert!(parse_hundredths(text).is_err(), "{text}");
        }
    }

    fn schedule(
        start: Option<&str>,
        duration: Option<i64>,
        milestone: bool,
    ) -> Result<Option<TicketSchedule>, CoreError> {
        resolve_schedule(Some(ScheduleInput {
            planned_start_at: start
                .map(|s| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)),
            duration_minutes: duration,
            is_schedule_milestone: milestone,
        }))
    }

    #[test]
    fn a_schedule_follows_the_duration_rules() {
        assert!(
            schedule(Some("2026-11-02T00:00:00Z"), Some(480), false)
                .unwrap()
                .is_some()
        );
        assert!(schedule(None, None, false).unwrap().is_none());
        assert!(schedule(None, Some(0), false).is_err());
        assert!(schedule(None, Some(-5), false).is_err());
        assert!(schedule(None, Some(MAX_MINUTES + 1), false).is_err());
        assert!(schedule(None, Some(0), true).unwrap().is_some());
        assert!(schedule(None, Some(30), true).is_err());
        assert!(schedule(Some("1999-01-01T00:00:00Z"), None, false).is_err());
        assert!(schedule(None, None, true).unwrap().is_some());
        assert!(resolve_schedule(None).unwrap().is_none());
    }
}
