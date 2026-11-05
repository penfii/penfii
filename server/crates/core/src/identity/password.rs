//! The rule for a password somebody chooses: when an account is created, the password is changed,
//! an invitation is accepted, or an operator sets one. It is not applied when a password is
//! checked at sign-in or re-authentication, which would lock out an account whose password was
//! set under an earlier rule.
//!
//! The rule follows NIST SP 800-63B: length is what counts (at least [`MIN_CHARS`], at most
//! [`MAX_CHARS`], each Unicode code point is one character), any character including whitespace is
//! allowed, there are no composition rules, no expiry, and a password that is known to be weak is
//! refused (a local denylist plus the account's own e-mail address). Nothing here stops a paste.

use crate::error::CoreError;

pub const MIN_CHARS: usize = 15;
pub const MAX_CHARS: usize = 128;

/// Lower-case passwords that every cracking dictionary tries first.
const COMMON: &str = include_str!("common_passwords.txt");

fn common() -> &'static std::collections::HashSet<&'static str> {
    static SET: std::sync::OnceLock<std::collections::HashSet<&'static str>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| {
        COMMON
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()
    })
}

/// Checks a password somebody is about to set. `email` is the account's address: a password built
/// from it is as guessable as the address itself.
pub fn validate_new_password(password: &str, email: &str) -> Result<(), CoreError> {
    let length = password.chars().count();
    if length < MIN_CHARS {
        return Err(CoreError::validation(format!(
            "password must be at least {MIN_CHARS} characters"
        )));
    }
    if length > MAX_CHARS {
        return Err(CoreError::validation(format!(
            "password must be at most {MAX_CHARS} characters"
        )));
    }
    if is_weak(password, email) {
        return Err(CoreError::validation(
            "password is too easy to guess; use a longer phrase that is not a common word or pattern",
        ));
    }
    Ok(())
}

/// Lower case with whitespace and the separators people put between words removed, so that
/// `Pass-Word Pass-Word` is seen as `passwordpassword`.
fn squash(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace() && !matches!(ch, '-' | '_' | '.' | ','))
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_weak(password: &str, email: &str) -> bool {
    let squashed = squash(password);
    let chars: Vec<char> = squashed.chars().collect();
    if chars.is_empty() {
        return true;
    }
    let list = common();
    if list.contains(squashed.as_str()) {
        return true;
    }
    // A common word followed only by digits or symbols: `password1234567890`.
    let stem = squashed.trim_end_matches(|ch: char| !ch.is_alphabetic());
    if !stem.is_empty() && stem != squashed && list.contains(stem) {
        return true;
    }
    // One short unit repeated: `aaaaaaaaaaaaaaaa`, `abababababababab`, `passwordpassword`.
    for unit in 1..=chars.len() / 2 {
        if chars.len().is_multiple_of(unit)
            && chars.chunks(unit).all(|chunk| chunk == &chars[..unit])
        {
            let unit_text: String = chars[..unit].iter().collect();
            if unit <= 4 || list.contains(unit_text.as_str()) {
                return true;
            }
        }
    }
    // A run such as `1234567890123456` or `abcdefghijklmnop`.
    if chars.len() >= 8 && is_run(&chars) {
        return true;
    }
    // Mostly built from words an attacker knows about the account: its address, or this service.
    // What is left once those are taken out has to be long enough to stand on its own.
    let mut rest = squashed.clone();
    for known in [
        squash(email),
        squash(email.split('@').next().unwrap_or_default()),
        "penfii".to_owned(),
    ] {
        if known.chars().count() >= 4 {
            rest = rest.replace(&known, "");
        }
    }
    if rest.chars().count() < 8 {
        return true;
    }
    false
}

/// Every character is the previous one plus or minus one (digits wrap around: `...890123...`).
fn is_run(chars: &[char]) -> bool {
    let step = |a: char, b: char| -> Option<i32> {
        if a.is_ascii_digit() && b.is_ascii_digit() {
            let diff = (b as i32 - a as i32).rem_euclid(10);
            return match diff {
                1 => Some(1),
                9 => Some(-1),
                _ => None,
            };
        }
        match b as i32 - a as i32 {
            1 => Some(1),
            -1 => Some(-1),
            _ => None,
        }
    };
    let Some(first) = step(chars[0], chars[1]) else {
        return false;
    };
    chars
        .windows(2)
        .all(|pair| step(pair[0], pair[1]) == Some(first))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EMAIL: &str = "dana.kim@example.com";

    fn refused(password: &str) -> bool {
        validate_new_password(password, EMAIL).is_err()
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        assert!(refused("short one 13 c"), "14 characters");
        assert!(!refused("exactly 15 char"), "15 characters is the minimum");
        assert!(!refused("this one is 15!!"));
        // 15 Hangul syllables are 45 bytes but 15 characters.
        assert!(!refused("보라색고양이가창가에서해를쬐고있다요"));
        assert!(refused("가나다라마바사아자차카타파하"), "14 syllables");
        assert!(refused(&"x".repeat(MAX_CHARS + 1)));
        let long = "a1b2".repeat(32); // 128 characters, not a repeated short word
        assert_eq!(long.chars().count(), MAX_CHARS);
        assert!(
            refused(&long),
            "a repeated unit is refused for being weak, not for length"
        );
        let ok = "correct-horse-staple-battery-9 lantern ".repeat(3);
        assert!(ok.chars().count() <= MAX_CHARS);
        assert!(!refused(&ok));
    }

    #[test]
    fn whitespace_and_symbols_are_allowed_and_nothing_is_required() {
        assert!(!refused("   lantern  orbit   plum   "));
        assert!(!refused("only lowercase words here"));
        assert!(!refused("ONLY UPPERCASE WORDS HERE"));
        assert!(!refused("3141592653589793238462"));
    }

    #[test]
    fn common_passwords_and_simple_patterns_are_refused() {
        for weak in [
            "passwordpassword",
            "Password Password",
            "password1234567890",
            "1234567890123456",
            "9876543210987654",
            "abcdefghijklmnop",
            "aaaaaaaaaaaaaaaa",
            "abababababababab",
            "qwertyuiopasdfghjkl",
            "iloveyouiloveyou",
            "letmeinletmeinletmein",
            "Correct-Horse-Battery-Staple".replace("-", "").as_str(),
            "비밀번호비밀번호비밀번호비밀번호",
        ] {
            assert!(refused(weak), "{weak}");
        }
    }

    #[test]
    fn a_password_made_from_the_address_is_refused() {
        assert!(refused("dana.kim@example.com"));
        assert!(refused("danakim-danakim-1"));
        assert!(refused("penfii penfii penfii"));
        // The address may appear, so long as the rest is a password in its own right.
        assert!(!refused("danakim lantern orbit plum"));
        assert!(!refused("lantern orbit plum dune"));
        assert!(validate_new_password("danakim-danakim-1", "someone@else.org").is_ok());
    }
}
