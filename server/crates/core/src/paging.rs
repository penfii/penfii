//! Paging helpers shared by the lists that continue from a `(timestamp, id)` position.
//!
//! A cursor is opaque to clients. Its text is `<time>|<id>` (the time with microseconds and a `Z`,
//! which is all PostgreSQL keeps), either as it is ([`encode_cursor`]) or hex-encoded so it is safe in a
//! URL as it stands ([`encode_cursor_hex`]). Decoding accepts any RFC 3339 time. A cursor that does not
//! parse is a validation error (`400`), never an empty page.
//!
//! A limit is either clamped into its range ([`clamp_limit`]) or rejected when it is outside it
//! ([`require_limit`]). Which one a list uses is part of its API and is kept as it was; the message of a
//! rejection is the list's own.
//!
//! Not here: the search cursor (bound to the query it came from and signed, `search::mod`) and the
//! schedule cursor (a plain offset, `project_schedule`).

use chrono::{DateTime, SecondsFormat, Utc};
use uuid::Uuid;

use crate::error::CoreError;

fn invalid_cursor() -> CoreError {
    CoreError::validation("invalid cursor")
}

/// `<time>|<id>`: the position after which the next page starts.
pub(crate) fn encode_cursor(at: DateTime<Utc>, id: Uuid) -> String {
    format!("{}|{id}", at.to_rfc3339_opts(SecondsFormat::Micros, true))
}

pub(crate) fn decode_cursor(text: &str) -> Result<(DateTime<Utc>, Uuid), CoreError> {
    let (at, id) = text.split_once('|').ok_or_else(invalid_cursor)?;
    let at = DateTime::parse_from_rfc3339(at)
        .map_err(|_| invalid_cursor())?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(id).map_err(|_| invalid_cursor())?;
    Ok((at, id))
}

/// [`encode_cursor`], hex-encoded.
pub(crate) fn encode_cursor_hex(at: DateTime<Utc>, id: Uuid) -> String {
    hex::encode(encode_cursor(at, id))
}

pub(crate) fn decode_cursor_hex(cursor: &str) -> Result<(DateTime<Utc>, Uuid), CoreError> {
    let bytes = hex::decode(cursor).map_err(|_| invalid_cursor())?;
    let text = String::from_utf8(bytes).map_err(|_| invalid_cursor())?;
    decode_cursor(&text)
}

/// The requested limit, or `default`, held to `1..=max`.
pub(crate) fn clamp_limit(limit: Option<i64>, default: i64, max: i64) -> i64 {
    limit.unwrap_or(default).clamp(1, max)
}

/// The requested limit, or `default`; outside `1..=max` it is rejected with `message`.
pub(crate) fn require_limit(
    limit: Option<i64>,
    default: i64,
    max: i64,
    message: &'static str,
) -> Result<i64, CoreError> {
    match limit.unwrap_or(default) {
        value if (1..=max).contains(&value) => Ok(value),
        _ => Err(CoreError::validation(message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-03-04T05:06:07.123456Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn a_cursor_round_trips_in_both_forms() {
        let id = Uuid::new_v4();
        let plain = encode_cursor(at(), id);
        assert_eq!(plain, format!("2026-03-04T05:06:07.123456Z|{id}"));
        assert_eq!(decode_cursor(&plain).unwrap(), (at(), id));
        assert_eq!(
            decode_cursor_hex(&encode_cursor_hex(at(), id)).unwrap(),
            (at(), id)
        );
    }

    #[test]
    fn a_cursor_with_any_rfc3339_offset_still_decodes() {
        let id = Uuid::new_v4();
        let old = format!("2026-03-04T05:06:07.123456+00:00|{id}");
        assert_eq!(decode_cursor(&old).unwrap(), (at(), id));
    }

    #[test]
    fn a_broken_cursor_is_a_validation_error() {
        let id = Uuid::new_v4();
        for bad in ["", "nonsense", "x|y", &format!("not-a-time|{id}")] {
            assert!(matches!(decode_cursor(bad), Err(CoreError::Validation(_))));
        }
        for bad in ["zz", "6e6f", &hex::encode("no-separator")] {
            assert!(matches!(
                decode_cursor_hex(bad),
                Err(CoreError::Validation(_))
            ));
        }
    }

    #[test]
    fn limits_clamp_or_reject_as_asked() {
        assert_eq!(clamp_limit(None, 20, 50), 20);
        assert_eq!(clamp_limit(Some(0), 20, 50), 1);
        assert_eq!(clamp_limit(Some(500), 20, 50), 50);
        assert_eq!(require_limit(None, 20, 50, "bad").unwrap(), 20);
        assert_eq!(require_limit(Some(50), 20, 50, "bad").unwrap(), 50);
        for out in [0, -1, 51] {
            assert!(require_limit(Some(out), 20, 50, "bad").is_err());
        }
    }
}
