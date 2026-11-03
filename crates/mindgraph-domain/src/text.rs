//! Shared validation for human-readable text fields.

use crate::DomainError;

/// Maximum length of names shown to people (workspace, graph, display names).
pub(crate) const MAX_DISPLAY_TEXT_CHARS: usize = 120;

/// Maximum length of free-form descriptions.
pub(crate) const MAX_DESCRIPTION_CHARS: usize = 2_000;

/// Trims `value` and checks it is non-empty and at most `max_chars` long.
pub(crate) fn required_text(
    field: &'static str,
    value: &str,
    max_chars: usize,
) -> Result<String, DomainError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DomainError::EmptyValue { field });
    }
    check_length(field, trimmed, max_chars)?;
    Ok(trimmed.to_owned())
}

/// Trims `value`; blank text becomes `None`. Non-blank text must be at most
/// `max_chars` long.
pub(crate) fn optional_text(
    field: &'static str,
    value: Option<&str>,
    max_chars: usize,
) -> Result<Option<String>, DomainError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(trimmed) => {
            check_length(field, trimmed, max_chars)?;
            Ok(Some(trimmed.to_owned()))
        }
    }
}

fn check_length(field: &'static str, value: &str, max_chars: usize) -> Result<(), DomainError> {
    if value.chars().count() > max_chars {
        return Err(DomainError::TooLong {
            field,
            max: max_chars,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_text_trims_and_rejects_blank() {
        assert_eq!(required_text("name", "  Fraud  ", 10).unwrap(), "Fraud");
        assert_eq!(
            required_text("name", " \t\n", 10),
            Err(DomainError::EmptyValue { field: "name" })
        );
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        // Five characters, fifteen bytes.
        assert!(required_text("name", "日本語ab", 5).is_ok());
        assert_eq!(
            required_text("name", "日本語abc", 5),
            Err(DomainError::TooLong {
                field: "name",
                max: 5
            })
        );
    }

    #[test]
    fn optional_text_maps_blank_to_none() {
        assert_eq!(optional_text("description", None, 10).unwrap(), None);
        assert_eq!(optional_text("description", Some("   "), 10).unwrap(), None);
        assert_eq!(
            optional_text("description", Some(" hi "), 10).unwrap(),
            Some("hi".to_owned())
        );
    }
}
