//! The search query language. Parsing never widens a search: anything it cannot read is an error.

use serde::Serialize;
use serde_json::{Value, json};

pub const KEYS: &[&str] = &[
    "in", "from", "assignee", "project", "team", "type", "status", "label", "before", "after",
    "has", "mentions",
];

pub const MAX_QUERY_CHARS: usize = 512;
pub const MAX_TOKENS: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Invalid {
    pub code: &'static str,
    pub token: String,
    pub message: String,
}

impl Invalid {
    pub fn new(code: &'static str, token: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            token: token.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Parsed {
    pub keywords: Vec<String>,
    pub phrases: Vec<String>,
    /// (key, value) in the order written. A repeated key means "any of".
    pub filters: Vec<(String, String)>,
}

impl Parsed {
    pub fn values(&self, key: &str) -> Vec<&str> {
        self.filters
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty() && self.phrases.is_empty() && self.filters.is_empty()
    }

    pub fn normalized(&self) -> Value {
        let mut filters = serde_json::Map::new();
        for (key, value) in &self.filters {
            filters
                .entry(key.clone())
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(json!(value));
        }
        json!({"keywords": self.keywords, "phrases": self.phrases, "filters": filters})
    }
}

/// Splits on whitespace; `"…"` groups; `key:value` and `key:"a b"` are filters.
pub fn parse(q: &str) -> Result<Parsed, Invalid> {
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(Invalid::new(
            "query_too_long",
            "",
            format!("a query is at most {MAX_QUERY_CHARS} characters"),
        ));
    }
    let chars: Vec<char> = q.chars().collect();
    let mut i = 0;
    let mut parsed = Parsed::default();
    let mut tokens = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        tokens += 1;
        if tokens > MAX_TOKENS {
            return Err(Invalid::new(
                "query_too_long",
                "",
                format!("a query has at most {MAX_TOKENS} words and filters"),
            ));
        }
        let start = i;
        // A bare quoted phrase.
        if chars[i] == '"' {
            let (text, next) = quoted(&chars, i)?;
            if !text.trim().is_empty() {
                parsed.phrases.push(text);
            }
            i = next;
            continue;
        }
        // Read a word up to whitespace, but let a quote after "key:" take over.
        let mut word = String::new();
        while i < chars.len() && !chars[i].is_whitespace() {
            if chars[i] == '"' && word.ends_with(':') {
                break;
            }
            word.push(chars[i]);
            i += 1;
        }
        if i < chars.len() && chars[i] == '"' {
            let (text, next) = quoted(&chars, i)?;
            i = next;
            let key = word.trim_end_matches(':').to_lowercase();
            push_filter(
                &mut parsed,
                &key,
                text,
                &chars[start..i].iter().collect::<String>(),
            )?;
            continue;
        }
        if let Some((key, value)) = word.split_once(':')
            && !key.is_empty()
            && key.chars().all(|c| c.is_ascii_alphabetic())
            && key.len() <= 12
        {
            let key = key.to_lowercase();
            if value.is_empty() {
                return Err(Invalid::new(
                    "invalid_value",
                    word.clone(),
                    "a filter needs a value",
                ));
            }
            push_filter(&mut parsed, &key, value.to_owned(), &word)?;
            continue;
        }
        if word.starts_with('-') && word.len() > 1 && word[1..].contains(':')
            || word.starts_with('(')
        {
            return Err(Invalid::new(
                "unsupported_syntax",
                word,
                "NOT, grouping and OR are not supported; list the filters you want",
            ));
        }
        parsed.keywords.push(word);
    }
    Ok(parsed)
}

fn quoted(chars: &[char], open: usize) -> Result<(String, usize), Invalid> {
    let mut i = open + 1;
    let mut text = String::new();
    while i < chars.len() {
        if chars[i] == '"' {
            return Ok((text, i + 1));
        }
        text.push(chars[i]);
        i += 1;
    }
    Err(Invalid::new(
        "unsupported_syntax",
        chars[open..].iter().collect::<String>(),
        "a quote was opened and not closed",
    ))
}

fn push_filter(parsed: &mut Parsed, key: &str, value: String, token: &str) -> Result<(), Invalid> {
    if key == "state" {
        return Err(Invalid::new(
            "unsupported_key",
            token,
            "a State belongs to one Project; choose it with the State chip, or use status: with not-started, in-progress, done or canceled",
        ));
    }
    if !KEYS.contains(&key) {
        return Err(Invalid::new(
            "unsupported_key",
            token,
            format!("unknown filter `{key}:`; quote the text to search for it"),
        ));
    }
    if value.trim().is_empty() {
        return Err(Invalid::new(
            "invalid_value",
            token,
            "a filter needs a value",
        ));
    }
    match key {
        "status" => {
            if status_category(&value).is_none() {
                return Err(Invalid::new(
                    "invalid_status",
                    token,
                    "status: takes not-started, in-progress, done or canceled",
                ));
            }
        }
        "has" => {
            if value.to_lowercase() != "file" {
                return Err(Invalid::new("invalid_value", token, "has: takes `file`"));
            }
        }
        "mentions" => {
            if value.to_lowercase() != "me" {
                return Err(Invalid::new("invalid_value", token, "mentions: takes `me`"));
            }
        }
        "before" | "after" if chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").is_err() => {
            return Err(Invalid::new(
                "invalid_date",
                token,
                "dates are written YYYY-MM-DD",
            ));
        }
        _ => {}
    }
    parsed.filters.push((key.to_owned(), value));
    Ok(())
}

pub fn status_category(word: &str) -> Option<&'static str> {
    match word.to_lowercase().replace('_', "-").as_str() {
        "not-started" => Some("NOT_STARTED"),
        "in-progress" => Some("IN_PROGRESS"),
        "done" => Some("DONE"),
        "canceled" | "cancelled" => Some("CANCELED"),
        _ => None,
    }
}

/// Which kinds can answer a filter. A kind without the field is left out of the search.
pub fn kind_supports(kind: &str, key: &str) -> bool {
    match key {
        "in" => matches!(kind, "message" | "ticket" | "file"),
        "from" => matches!(kind, "message" | "ticket" | "file" | "view" | "doc"),
        "assignee" | "type" | "status" | "label" => kind == "ticket",
        "project" | "team" => matches!(kind, "ticket" | "space" | "view" | "message" | "doc"),
        "before" | "after" => true,
        "has" => matches!(kind, "message" | "file"),
        "mentions" => kind == "message",
        _ => false,
    }
}

pub fn looks_like_ticket_key(word: &str) -> bool {
    let Some((prefix, number)) = word.rsplit_once('-') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_alphanumeric())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_phrases_and_filters() {
        let p =
            parse(r#"배포 "release notes" from:me project:PEN project:API status:done"#).unwrap();
        assert_eq!(p.keywords, vec!["배포"]);
        assert_eq!(p.phrases, vec!["release notes"]);
        assert_eq!(p.values("project"), vec!["PEN", "API"]);
        assert_eq!(p.values("status"), vec!["done"]);
    }

    #[test]
    fn quoted_values_and_times_stay_text() {
        let p = parse(r#"in:"my channel" 12:30"#).unwrap();
        assert_eq!(p.values("in"), vec!["my channel"]);
        assert_eq!(p.keywords, vec!["12:30"]);
    }

    #[test]
    fn invalid_input_is_an_error() {
        assert_eq!(parse("state:Review").unwrap_err().code, "unsupported_key");
        assert_eq!(parse("foo:bar").unwrap_err().code, "unsupported_key");
        assert_eq!(parse("status:review").unwrap_err().code, "invalid_status");
        assert_eq!(parse("before:yesterday").unwrap_err().code, "invalid_date");
        assert_eq!(parse("has:image").unwrap_err().code, "invalid_value");
        assert_eq!(parse("mentions:bob").unwrap_err().code, "invalid_value");
        assert_eq!(parse("-from:me").unwrap_err().code, "unsupported_syntax");
        assert_eq!(parse("(a b)").unwrap_err().code, "unsupported_syntax");
        assert_eq!(parse("\"open").unwrap_err().code, "unsupported_syntax");
        assert_eq!(parse("from:").unwrap_err().code, "invalid_value");
        assert_eq!(parse(&"a ".repeat(25)).unwrap_err().code, "query_too_long");
    }

    #[test]
    fn or_and_not_are_plain_words() {
        let p = parse("a OR b NOT c").unwrap();
        assert_eq!(p.keywords, vec!["a", "OR", "b", "NOT", "c"]);
    }

    #[test]
    fn status_words() {
        assert_eq!(status_category("In_Progress"), Some("IN_PROGRESS"));
        assert_eq!(status_category("done"), Some("DONE"));
        assert_eq!(status_category("review"), None);
    }

    #[test]
    fn ticket_keys() {
        assert!(looks_like_ticket_key("PEN-12"));
        assert!(!looks_like_ticket_key("pen-x"));
        assert!(!looks_like_ticket_key("-12"));
    }
}
