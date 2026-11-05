//! Project Ticket Prefix and the display Key `<PREFIX>-<sequence>`.
//!
//! The Ticket's internal id never changes. The Key is a separate, human-readable value built
//! from the Project's fixed Prefix and a per-Project counter that only moves forward.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::CoreError;

pub const PREFIX_MIN: usize = 2;
pub const PREFIX_MAX: usize = 10;

/// Canonical form of a Prefix: upper case, `A-Z` first, then `A-Z` or digits, 2..=10 long.
pub fn canonical_prefix(input: &str) -> Result<String, CoreError> {
    let value = input.trim().to_ascii_uppercase();
    let mut chars = value.chars();
    let valid = (PREFIX_MIN..=PREFIX_MAX).contains(&value.len())
        && chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if !valid {
        return Err(CoreError::validation(format!(
            "ticketPrefix must be {PREFIX_MIN}-{PREFIX_MAX} letters or digits and start with a letter"
        )));
    }
    Ok(value)
}

/// A short Prefix an author can recognize in the Project name, or `None` when the name has no
/// usable ASCII letters (the person then types one). One word gives its first three letters;
/// several words give their initials, topped up from the last word to reach three.
pub fn suggest_prefix(name: &str) -> Option<String> {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_uppercase)
        .collect();
    let first = words.first()?;
    if !first.chars().next()?.is_ascii_uppercase() {
        return None;
    }
    let mut prefix: String = if words.len() == 1 {
        first.chars().take(3).collect()
    } else {
        words
            .iter()
            .take(5)
            .filter_map(|word| word.chars().next())
            .collect()
    };
    if let Some(last) = words.last().filter(|_| words.len() > 1) {
        for c in last.chars().skip(1) {
            if prefix.len() >= 3 {
                break;
            }
            prefix.push(c);
        }
    }
    (prefix.len() >= PREFIX_MIN && prefix.chars().next()?.is_ascii_uppercase()).then_some(prefix)
}

/// `PFI-001`; three digits is the minimum width, not a limit.
pub fn format_key(prefix: &str, sequence: i64) -> String {
    format!("{prefix}-{sequence:03}")
}

pub async fn prefix_taken(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    prefix: &str,
) -> Result<bool, CoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM penfii.projects WHERE workspace_id=$1 AND lower(ticket_prefix)=lower($2))",
    )
    .bind(workspace_id)
    .bind(prefix)
    .fetch_one(&mut *conn)
    .await?)
}

/// The suggestion for a name, made unique inside the workspace by a numeric tail when needed.
pub async fn available_suggestion(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    name: &str,
) -> Result<Option<String>, CoreError> {
    let Some(base) = suggest_prefix(name) else {
        return Ok(None);
    };
    unique_variant(conn, workspace_id, &base).await.map(Some)
}

async fn unique_variant(
    conn: &mut PgConnection,
    workspace_id: Uuid,
    base: &str,
) -> Result<String, CoreError> {
    if !prefix_taken(conn, workspace_id, base).await? {
        return Ok(base.to_owned());
    }
    for n in 2..1000 {
        let tail = n.to_string();
        let head: String = base.chars().take(PREFIX_MAX - tail.len()).collect();
        let candidate = format!("{head}{tail}");
        if !prefix_taken(conn, workspace_id, &candidate).await? {
            return Ok(candidate);
        }
    }
    Err(CoreError::conflict(
        "ticket_prefix_taken",
        "no free ticket prefix for this name",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_are_canonical_and_bounded() {
        assert_eq!(canonical_prefix(" pfi ").unwrap(), "PFI");
        assert_eq!(canonical_prefix("Pfi2").unwrap(), "PFI2");
        for bad in ["", "P", "1AB", "AB-C", "AB C", "ABCDEFGHIJK", "가나다"] {
            assert!(canonical_prefix(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn suggestions_come_from_the_name() {
        assert_eq!(suggest_prefix("Penfii").as_deref(), Some("PEN"));
        assert_eq!(suggest_prefix("Customer Portal").as_deref(), Some("CPO"));
        assert_eq!(suggest_prefix("Penfii Backend").as_deref(), Some("PBA"));
        assert_eq!(
            suggest_prefix("web app v2 launch plan").as_deref(),
            Some("WAVLP")
        );
        assert_eq!(suggest_prefix("페니파이"), None);
        assert_eq!(suggest_prefix("2024 plan"), None);
        assert_eq!(suggest_prefix("   "), None);
    }

    #[test]
    fn keys_pad_to_three_digits_and_grow_past_999() {
        assert_eq!(format_key("PFI", 1), "PFI-001");
        assert_eq!(format_key("PFI", 12), "PFI-012");
        assert_eq!(format_key("PFI", 999), "PFI-999");
        assert_eq!(format_key("PFI", 1000), "PFI-1000");
    }
}
