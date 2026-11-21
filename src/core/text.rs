//! Shared checks for short free-text fields, and how text from the other
//! side is written to logs and the terminal.

use std::fmt::{self, Write};

use rootcause::option_ext::NoneError;

use crate::{Report, core::invalid::Invalid};

/// Text from the other side, for logs and the terminal: what the share
/// sends a joiner (its lines, responses and pages, and errors quoting
/// them), or a joiner the share. Shown with its control characters, and
/// the characters that reorder text, escaped (`\u{1b}`, `\r`, `\n`), so
/// it can neither forge log lines nor drive the terminal. Every log or
/// print of such text goes through this.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Untrusted<T>(pub(crate) T);

impl<T: fmt::Display> fmt::Display for Untrusted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(Escaping(f), "{}", self.0)
    }
}

/// A report for logs and the terminal: escaped contexts, outermost first,
/// without the diagnostic tree, source locations or `Option`'s marker.
pub(crate) struct OneLineReport<'a>(pub(crate) &'a Report);

impl fmt::Display for OneLineReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for report in self.0.iter_reports() {
            if report.downcast_current_context::<NoneError>().is_some() {
                continue;
            }
            write!(
                f,
                "{separator}{}",
                Untrusted(report.format_current_context())
            )?;
            separator = ": ";
        }
        Ok(())
    }
}

/// Writes to the formatter with unsafe characters escaped.
struct Escaping<'a, 'b>(&'a mut fmt::Formatter<'b>);

impl Write for Escaping<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for character in text.chars() {
            if escaped(character) {
                write!(self.0, "{}", character.escape_default())?;
            } else {
                self.0.write_char(character)?;
            }
        }
        Ok(())
    }
}

/// Control characters, the bidirectional controls that make text read
/// other than it is, and the Unicode line and paragraph separators.
pub(crate) fn escaped(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

/// Checks that `value` is not blank, has at most `max_chars` characters and
/// contains no control characters (newlines included).
pub(crate) fn check_line(value: &str, field: &str, max_chars: usize) -> Result<(), Invalid> {
    check_text(value, field, max_chars)?;
    if value.chars().any(char::is_control) {
        return Err(Invalid::new(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(())
}

/// Checks that `value` is not blank and has at most `max_chars` characters.
pub(crate) fn check_text(value: &str, field: &str, max_chars: usize) -> Result<(), Invalid> {
    if value.trim().is_empty() {
        return Err(Invalid::new(format!("{field} must not be blank")));
    }
    if value.chars().count() > max_chars {
        return Err(Invalid::new(format!(
            "{field} must be at most {max_chars} characters"
        )));
    }
    Ok(())
}

/// Checks only the length of `value`; empty is allowed.
pub(crate) fn check_len(value: &str, field: &str, max_chars: usize) -> Result<(), Invalid> {
    if value.chars().count() > max_chars {
        return Err(Invalid::new(format!(
            "{field} must be at most {max_chars} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::text::{OneLineReport, Untrusted, check_len, check_line, check_text};

    #[test]
    fn untrusted_text_has_its_controls_escaped() {
        let shown = Untrusted("a\r\nFORGED\u{1b}[2J\u{202e}b\tc é").to_string();
        assert_eq!(shown, "a\\r\\nFORGED\\u{1b}[2J\\u{202e}b\\tc é");
    }

    #[test]
    fn reports_show_only_the_escaped_context_chain() {
        use rootcause::{option_ext::OptionExt, prelude::report};

        let error: crate::Report = report!("permission denied\nFORGED")
            .context("could not read file")
            .into();
        let shown = OneLineReport(&error).to_string();
        assert_eq!(shown, r"could not read file: permission denied\nFORGED");
        assert!(!shown.contains('\n'));
        assert!(!shown.contains("src/"));
        let missing: crate::Report = None::<()>.context("the share stopped").unwrap_err().into();
        assert_eq!(OneLineReport(&missing).to_string(), "the share stopped");
    }

    #[test]
    fn lines_and_texts() {
        assert!(check_line("ok", "f", 2).is_ok());
        assert!(check_line("too long", "f", 2).is_err());
        assert!(check_line("  ", "f", 2).is_err());
        assert!(check_line("a\nb", "f", 10).is_err());
        assert!(check_text("a\nb", "f", 10).is_ok());
        assert!(check_len("", "f", 0).is_ok());
        assert!(check_len("é", "f", 1).is_ok());
    }
}
