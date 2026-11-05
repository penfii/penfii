//! Notification preferences, channel mute and Do Not Disturb.
//!
//! Do Not Disturb never removes or delays an Inbox record. It describes when delivery *outside*
//! the app must wait; there is no such delivery yet, so today it only tells the client to stay quiet.

use chrono::{DateTime, Datelike, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::access;
use crate::error::CoreError;
use crate::visibility;

pub const DIRECT_KINDS: &[&str] = &[
    "ticket.assigned",
    "mention.user",
    "dependency.released",
    "reply.direct",
    "reminder.due",
];

pub fn is_watcher_kind(kind: &str) -> bool {
    !DIRECT_KINDS.contains(&kind)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dnd {
    pub enabled: bool,
    /// Weekly quiet hours in the person's own time zone.
    pub schedule: Option<Schedule>,
    /// Quiet until this moment, whatever the schedule says.
    pub snooze_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    /// 0 = Monday … 6 = Sunday
    pub days: Vec<u32>,
    pub start: String,
    pub end: String,
    pub time_zone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    /// `ALL` or `DIRECT_ONLY` (mentions, replies, assignment, released dependencies, reminders).
    pub level: String,
    pub dnd: Dnd,
    /// Reserved for outside delivery. Nothing reads it yet.
    pub external: Value,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            level: "ALL".into(),
            dnd: Dnd {
                enabled: false,
                schedule: None,
                snooze_until: None,
            },
            external: json!({}),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesView {
    #[serde(flatten)]
    pub preferences: Preferences,
    pub version: i64,
    pub dnd_active: bool,
}

fn parse_time(text: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(text, "%H:%M").ok()
}

/// Whether quiet hours apply at `now`. `offset_minutes` is the person's UTC offset at `now`.
pub fn dnd_active(dnd: &Dnd, now: DateTime<Utc>, offset_minutes: i32) -> bool {
    if !dnd.enabled {
        return false;
    }
    if let Some(until) = dnd.snooze_until
        && until > now
    {
        return true;
    }
    let Some(schedule) = &dnd.schedule else {
        // Enabled with no schedule means quiet until turned off.
        return dnd.snooze_until.is_none();
    };
    let (Some(start), Some(end)) = (parse_time(&schedule.start), parse_time(&schedule.end)) else {
        return false;
    };
    let local = now + chrono::Duration::minutes(i64::from(offset_minutes));
    let time = local.time();
    let weekday = local.weekday().num_days_from_monday();
    let today = schedule.days.contains(&weekday);
    let yesterday = schedule.days.contains(&((weekday + 6) % 7));
    if start <= end {
        today && time >= start && time < end
    } else {
        // Crosses midnight: the evening belongs to today, the early morning to yesterday's window.
        (today && time >= start) || (yesterday && time < end)
    }
}

fn validate(preferences: &Preferences) -> Result<(), CoreError> {
    if preferences.level != "ALL" && preferences.level != "DIRECT_ONLY" {
        return Err(CoreError::validation("level must be ALL or DIRECT_ONLY"));
    }
    if let Some(schedule) = &preferences.dnd.schedule {
        if parse_time(&schedule.start).is_none() || parse_time(&schedule.end).is_none() {
            return Err(CoreError::validation("quiet hours are written HH:MM"));
        }
        if schedule.days.iter().any(|d| *d > 6) || schedule.days.is_empty() {
            return Err(CoreError::validation("choose at least one day, 0 to 6"));
        }
        if schedule.time_zone.len() > 64 {
            return Err(CoreError::validation("time zone is too long"));
        }
    }
    if !preferences.external.is_object() {
        return Err(CoreError::validation("external must be an object"));
    }
    Ok(())
}

pub async fn load(conn: &mut PgConnection, user_id: Uuid) -> Result<(Preferences, i64), CoreError> {
    let row = sqlx::query(
        "SELECT settings, version FROM penfii.notification_preferences WHERE user_id=$1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match row {
        Some(row) => {
            let settings: Value = row.try_get("settings")?;
            (
                serde_json::from_value(settings).unwrap_or_default(),
                row.try_get("version")?,
            )
        }
        None => (Preferences::default(), 0),
    })
}

async fn offset_minutes(conn: &mut PgConnection, time_zone: &str, now: DateTime<Utc>) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "SELECT (extract(epoch FROM (($1::timestamptz AT TIME ZONE $2) - ($1::timestamptz AT TIME ZONE 'UTC'))) / 60)::int",
    )
    .bind(now)
    .bind(time_zone)
    .fetch_one(&mut *conn)
    .await
    .unwrap_or(0)
}

pub async fn view(pool: &PgPool, actor: Uuid) -> Result<PreferencesView, CoreError> {
    let mut conn = pool.acquire().await?;
    let (preferences, version) = load(&mut conn, actor).await?;
    let now = Utc::now();
    let zone = preferences
        .dnd
        .schedule
        .as_ref()
        .map(|s| s.time_zone.clone())
        .unwrap_or_else(|| "UTC".into());
    let offset = offset_minutes(&mut conn, &zone, now).await;
    Ok(PreferencesView {
        dnd_active: dnd_active(&preferences.dnd, now, offset),
        preferences,
        version,
    })
}

pub async fn save(
    pool: &PgPool,
    actor: Uuid,
    preferences: Preferences,
    expected_version: i64,
) -> Result<PreferencesView, CoreError> {
    validate(&preferences)?;
    let mut tx = pool.begin().await?;
    if let Some(schedule) = &preferences.dnd.schedule {
        let known: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_timezone_names WHERE name=$1)")
                .bind(&schedule.time_zone)
                .fetch_one(&mut *tx)
                .await?;
        if !known {
            return Err(CoreError::validation("unknown time zone"));
        }
    }
    let settings =
        serde_json::to_value(&preferences).map_err(|e| CoreError::Internal(e.to_string()))?;
    let updated = if expected_version == 0 {
        sqlx::query(
            "INSERT INTO penfii.notification_preferences(user_id, settings, version) VALUES($1,$2,1) ON CONFLICT DO NOTHING",
        )
        .bind(actor)
        .bind(&settings)
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        sqlx::query(
            "UPDATE penfii.notification_preferences SET settings=$2, version=version+1, updated_at=now() WHERE user_id=$1 AND version=$3",
        )
        .bind(actor)
        .bind(&settings)
        .bind(expected_version)
        .execute(&mut *tx)
        .await?
        .rows_affected()
    };
    if updated == 0 {
        let current: i64 = sqlx::query_scalar("SELECT coalesce((SELECT version FROM penfii.notification_preferences WHERE user_id=$1),0)")
            .bind(actor)
            .fetch_one(&mut *tx)
            .await?;
        let mut extra = serde_json::Map::new();
        extra.insert("currentVersion".into(), json!(current));
        return Err(CoreError::conflict_extra(
            "version_conflict",
            "preferences changed elsewhere",
            extra,
        ));
    }
    tx.commit().await?;
    view(pool, actor).await
}

/// Mute (`on`) or unmute a channel for the person. Muting needs only that they can see the channel.
pub async fn set_mute(
    pool: &PgPool,
    actor: Uuid,
    channel_id: Uuid,
    on: bool,
) -> Result<bool, CoreError> {
    let mut tx = pool.begin().await?;
    let workspace: Uuid = sqlx::query_scalar(
        "SELECT c.workspace_id FROM penfii.channels ch JOIN penfii.conversations c ON c.id=ch.id WHERE ch.id=$1",
    )
    .bind(channel_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(CoreError::NotFound)?;
    access::require_member(&mut tx, workspace, actor).await?;
    let readable = access::load_conversation(&mut tx, actor, channel_id)
        .await?
        .is_some_and(|a| a.can_read())
        || visibility::channels(&mut tx, actor, &[channel_id])
            .await?
            .contains(&channel_id);
    if !readable {
        return Err(CoreError::NotFound);
    }
    if on {
        sqlx::query("INSERT INTO penfii.channel_mutes(user_id, channel_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(actor)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("DELETE FROM penfii.channel_mutes WHERE user_id=$1 AND channel_id=$2")
            .bind(actor)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(on)
}

pub async fn muted_channels(pool: &PgPool, actor: Uuid) -> Result<Vec<Uuid>, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT channel_id FROM penfii.channel_mutes WHERE user_id=$1 ORDER BY created_at",
    )
    .bind(actor)
    .fetch_all(pool)
    .await?)
}

pub async fn is_muted(
    conn: &mut PgConnection,
    user_id: Uuid,
    channel_id: Uuid,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.channel_mutes WHERE user_id=$1 AND channel_id=$2)",
    )
    .bind(user_id)
    .bind(channel_id)
    .fetch_one(&mut *conn)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dnd(start: &str, end: &str, days: Vec<u32>) -> Dnd {
        Dnd {
            enabled: true,
            schedule: Some(Schedule {
                days,
                start: start.into(),
                end: end.into(),
                time_zone: "Asia/Seoul".into(),
            }),
            snooze_until: None,
        }
    }

    #[test]
    fn quiet_hours_cross_midnight_and_follow_the_persons_clock() {
        let night = dnd("22:00", "07:00", vec![0, 1, 2, 3, 4]);
        // Monday 2026-10-19 23:30 in Seoul (UTC+9) is 14:30 UTC.
        let monday_late = Utc.with_ymd_and_hms(2026, 10, 19, 14, 30, 0).unwrap();
        assert!(dnd_active(&night, monday_late, 540));
        // Tuesday 06:00 Seoul belongs to Monday's window.
        let tuesday_early = Utc.with_ymd_and_hms(2026, 10, 19, 21, 0, 0).unwrap();
        assert!(dnd_active(&night, tuesday_early, 540));
        // Tuesday 12:00 Seoul is outside.
        let noon = Utc.with_ymd_and_hms(2026, 10, 20, 3, 0, 0).unwrap();
        assert!(!dnd_active(&night, noon, 540));
        // Saturday night is not in the window (Saturday is day 5).
        let saturday = Utc.with_ymd_and_hms(2026, 10, 24, 14, 30, 0).unwrap();
        assert!(!dnd_active(&night, saturday, 540));
        // Sunday 06:00 belongs to Saturday, which is not a quiet day.
        let sunday_early = Utc.with_ymd_and_hms(2026, 10, 24, 21, 0, 0).unwrap();
        assert!(!dnd_active(&night, sunday_early, 540));
    }

    #[test]
    fn disabled_snoozed_and_scheduleless() {
        let mut d = dnd("09:00", "10:00", vec![0]);
        d.enabled = false;
        assert!(!dnd_active(&d, Utc::now(), 0));
        let mut d = Dnd {
            enabled: true,
            schedule: None,
            snooze_until: None,
        };
        assert!(dnd_active(&d, Utc::now(), 0));
        d.snooze_until = Some(Utc::now() + chrono::Duration::hours(1));
        assert!(dnd_active(&d, Utc::now(), 0));
        d.snooze_until = Some(Utc::now() - chrono::Duration::hours(1));
        assert!(!dnd_active(&d, Utc::now(), 0));
    }
}
