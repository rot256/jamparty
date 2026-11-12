//! Shared checks for short free-text fields.

use crate::core::invalid::Invalid;

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
    use crate::core::text::{check_len, check_line, check_text};

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
