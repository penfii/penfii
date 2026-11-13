//! Where the operator's password comes from. A value on the command line (`--password`) shows up in
//! the shell history and in the process list, so the commands that set a password can also read it
//! from standard input (`--password-stdin`) or from a hidden prompt. All three commands go through
//! [`resolve`]; the value is never written to an error message.

use std::io::{self, BufRead, IsTerminal, Read, Write};

/// The core limit for a password that is checked; reading stops being reasonable beyond it.
const MAX_BYTES: usize = 1024;

#[derive(clap::Args, Debug, Clone)]
pub struct PasswordArgs {
    /// The password as an argument. It stays in the shell history and the process list; prefer
    /// --password-stdin or the prompt.
    #[arg(long, conflicts_with = "password_stdin")]
    password: Option<String>,
    /// Read the password from standard input (one line) instead of the command line.
    #[arg(long)]
    password_stdin: bool,
}

/// Reads one line from `input`: drops one trailing `\n` or `\r\n`, keeps every other character,
/// refuses an empty or oversized value and anything that is not UTF-8.
pub fn read_line<R: BufRead>(input: &mut R) -> Result<String, String> {
    let mut raw = Vec::new();
    // One byte past the limit tells "exactly at the limit" apart from "too long".
    input
        .by_ref()
        .take(MAX_BYTES as u64 + 2)
        .read_until(b'\n', &mut raw)
        .map_err(|error| format!("could not read the password: {}", error.kind()))?;
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
    if raw.len() > MAX_BYTES {
        return Err(format!("the password is longer than {MAX_BYTES} bytes"));
    }
    let text = String::from_utf8(raw).map_err(|_| "the password is not valid UTF-8".to_string())?;
    if text.is_empty() {
        return Err("the password is empty".to_string());
    }
    Ok(text)
}

/// Asks twice through `ask` and returns the value only when both answers are the same.
pub fn confirmed<F>(mut ask: F) -> Result<String, String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    let first = ask("New password: ")?;
    let again = ask("Repeat the password: ")?;
    if first != again {
        return Err("the two passwords differ; nothing was changed".to_string());
    }
    Ok(first)
}

/// Chooses the source: the flag (with a warning), standard input, or the prompt on a terminal.
/// `stdin_is_terminal` and the readers are injected so the choice can be tested without a TTY.
pub fn resolve_with<R, F>(
    args: &PasswordArgs,
    stdin: &mut R,
    stdin_is_terminal: bool,
    warn: &mut dyn Write,
    ask: F,
) -> Result<String, String>
where
    R: BufRead,
    F: FnMut(&str) -> Result<String, String>,
{
    if let Some(value) = &args.password {
        let _ = writeln!(
            warn,
            "warning: --password leaves the password in the shell history and the process list; use --password-stdin or leave it out to be prompted"
        );
        return Ok(value.clone());
    }
    if args.password_stdin {
        return read_line(stdin);
    }
    if !stdin_is_terminal {
        return Err(
            "no password: give --password-stdin (and pipe the password in) or run from a terminal to be prompted"
                .to_string(),
        );
    }
    confirmed(ask)
}

pub fn resolve(args: &PasswordArgs) -> Result<String, String> {
    let terminal = io::stdin().is_terminal();
    if args.password.is_none() && !args.password_stdin {
        // The prompt reads standard input itself, line by line; nothing may hold its lock meanwhile.
        return resolve_with(
            args,
            &mut io::BufReader::new(io::empty()),
            terminal,
            &mut io::stderr(),
            prompt_hidden,
        );
    }
    let mut lock = io::stdin().lock();
    resolve_with(args, &mut lock, terminal, &mut io::stderr(), prompt_hidden)
}

/// Prompts on stderr and reads one line with echo turned off (restored on every way out).
fn prompt_hidden(label: &str) -> Result<String, String> {
    let mut err = io::stderr();
    let _ = write!(err, "{label}");
    let _ = err.flush();
    let guard = echo::Off::new();
    let result = read_line(&mut io::stdin().lock());
    drop(guard);
    let _ = writeln!(err);
    result
}

#[cfg(unix)]
mod echo {
    /// Clears ECHO on standard input and puts the old settings back when dropped.
    pub struct Off(Option<libc::termios>);

    impl Off {
        pub fn new() -> Self {
            // SAFETY: tcgetattr/tcsetattr only read and write the `termios` struct we own.
            unsafe {
                let mut term: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(libc::STDIN_FILENO, &mut term) != 0 {
                    return Self(None);
                }
                let saved = term;
                term.c_lflag &= !libc::ECHO;
                if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &term) != 0 {
                    return Self(None);
                }
                Self(Some(saved))
            }
        }
    }

    impl Drop for Off {
        fn drop(&mut self) {
            if let Some(saved) = self.0 {
                // SAFETY: restores the settings read in `new`.
                unsafe {
                    libc::tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, &saved);
                }
            }
        }
    }
}

#[cfg(not(unix))]
mod echo {
    pub struct Off;
    impl Off {
        pub fn new() -> Self {
            Self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn args(password: Option<&str>, stdin: bool) -> PasswordArgs {
        PasswordArgs {
            password: password.map(str::to_string),
            password_stdin: stdin,
        }
    }

    fn no_prompt(_: &str) -> Result<String, String> {
        panic!("must not prompt")
    }

    #[test]
    fn a_line_loses_one_line_ending_and_nothing_else() {
        assert_eq!(read_line(&mut Cursor::new("a b c \n")).unwrap(), "a b c ");
        assert_eq!(read_line(&mut Cursor::new("pass\r\n")).unwrap(), "pass");
        assert_eq!(
            read_line(&mut Cursor::new("no newline")).unwrap(),
            "no newline"
        );
        assert_eq!(
            read_line(&mut Cursor::new("한글 비밀번호\n")).unwrap(),
            "한글 비밀번호"
        );
        // Only the first line counts.
        assert_eq!(read_line(&mut Cursor::new("one\ntwo\n")).unwrap(), "one");
    }

    #[test]
    fn empty_oversized_and_non_utf8_input_is_refused_without_echoing_it() {
        assert!(read_line(&mut Cursor::new("")).is_err());
        assert!(read_line(&mut Cursor::new("\n")).is_err());
        assert!(read_line(&mut Cursor::new("\r\n")).is_err());
        let long = "x".repeat(MAX_BYTES + 1);
        let error = read_line(&mut Cursor::new(format!("{long}\n"))).unwrap_err();
        assert!(
            !error.contains("xxx"),
            "the value must not be printed: {error}"
        );
        assert!(read_line(&mut Cursor::new(vec![0xff, 0xfe, b'\n'])).is_err());
        // Exactly at the limit is fine.
        let at_limit = "y".repeat(MAX_BYTES);
        assert_eq!(
            read_line(&mut Cursor::new(format!("{at_limit}\n")))
                .unwrap()
                .len(),
            MAX_BYTES
        );
    }

    #[test]
    fn the_flag_still_works_and_warns() {
        let mut warn = Vec::new();
        let got = resolve_with(
            &args(Some("value"), false),
            &mut Cursor::new(""),
            false,
            &mut warn,
            no_prompt,
        )
        .unwrap();
        assert_eq!(got, "value");
        let text = String::from_utf8(warn).unwrap();
        assert!(text.contains("--password-stdin"));
        assert!(
            !text.contains("value"),
            "the warning must not repeat the password"
        );
    }

    #[test]
    fn stdin_is_read_without_a_prompt_and_without_a_warning() {
        let mut warn = Vec::new();
        let got = resolve_with(
            &args(None, true),
            &mut Cursor::new("piped phrase\n"),
            true,
            &mut warn,
            no_prompt,
        )
        .unwrap();
        assert_eq!(got, "piped phrase");
        assert!(warn.is_empty());
        assert!(
            resolve_with(
                &args(None, true),
                &mut Cursor::new(""),
                false,
                &mut warn,
                no_prompt
            )
            .is_err()
        );
    }

    #[test]
    fn with_no_option_and_no_terminal_it_fails_instead_of_waiting() {
        let mut warn = Vec::new();
        let error = resolve_with(
            &args(None, false),
            &mut Cursor::new("ignored\n"),
            false,
            &mut warn,
            no_prompt,
        )
        .unwrap_err();
        assert!(error.contains("--password-stdin"));
    }

    #[test]
    fn the_prompt_asks_twice_and_refuses_a_mismatch() {
        let mut warn = Vec::new();
        let mut answers = vec!["same phrase".to_string(), "same phrase".to_string()].into_iter();
        let got = resolve_with(
            &args(None, false),
            &mut Cursor::new(""),
            true,
            &mut warn,
            |_| Ok(answers.next().unwrap()),
        )
        .unwrap();
        assert_eq!(got, "same phrase");
        let mut answers = vec!["alpha-secret".to_string(), "beta-secret".to_string()].into_iter();
        let error = confirmed(|_| Ok(answers.next().unwrap())).unwrap_err();
        assert!(error.contains("differ"));
        assert!(!error.contains("alpha-secret") && !error.contains("beta-secret"));
    }
}
