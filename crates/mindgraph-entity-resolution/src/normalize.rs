//! Deterministic value normalisation for candidate discovery (M9).
//!
//! Normalised values are comparison keys only. They are never written back to
//! an entity and never stored: the original property values stay untouched in
//! the GraphStore.
//!
//! - **Text**: Unicode NFKC, default case folding, NFKC again, then trimmed
//!   with every run of whitespace collapsed to one space. Stable and
//!   idempotent.
//! - **Names**: normalised text with punctuation (`.`, `,`, `-`, …) treated
//!   as a token separator and apostrophes removed, so `M. Ikhwan` gives the
//!   tokens `m`, `ikhwan`.
//! - **Email**: trimmed; the address must have a basic `local@domain.tld`
//!   shape; only the domain is lower-cased. The local part is kept as written
//!   because RFC 5321 lets a server treat it case-sensitively, and no
//!   provider-specific alias rule (dots, `+tag`) is applied.
//! - **Phone**: parsed with libphonenumber metadata (the `phonenumber`
//!   crate) and formatted as E.164. A national-format number (`0812…`) is
//!   only parsed when the rule names a default country; without one it is
//!   reported as unusable instead of being guessed.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use caseless::Caseless;

use crate::SignalNote;

/// Longest text (in characters) normalised for matching; longer values are
/// reported as [`SignalNote::ValueTooLong`].
pub const MAX_TEXT_CHARS: usize = 256;
/// Most tokens a name may have.
pub const MAX_NAME_TOKENS: usize = 16;
/// Longest email address accepted (RFC 5321 path limit).
pub const MAX_EMAIL_CHARS: usize = 254;
/// Longest phone number text accepted.
pub const MAX_PHONE_CHARS: usize = 40;

/// The result of normalising one property value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Normalized<T> {
    /// No value: absent, blank or only whitespace. Never matches.
    Missing,
    /// A value is present but cannot be compared, for example an invalid
    /// email address. Never matches.
    Unusable(SignalNote),
    /// The comparison key.
    Value(T),
}

impl<T> Normalized<T> {
    /// The key, if there is one.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }
}

/// NFKC, then default case folding, then NFKC, until stable (at most three
/// rounds; in practice one).
fn fold(value: &str) -> String {
    let mut current: String = value.nfkc().default_case_fold().nfkc().collect();
    for _ in 0..2 {
        let next: String = current.nfkc().default_case_fold().nfkc().collect();
        if next == current {
            break;
        }
        current = next;
    }
    current
}

/// The normalised form of free text: NFKC + case folding, trimmed, with
/// whitespace runs collapsed to one space. `None` for blank text.
///
/// ```
/// use mindgraph_entity_resolution::normalize_text;
///
/// assert_eq!(normalize_text("  Muhammad   IKHWAN \t"), Some("muhammad ikhwan".to_owned()));
/// assert_eq!(normalize_text(" \n "), None);
/// ```
#[must_use]
pub fn normalize_text(value: &str) -> Option<String> {
    let folded = fold(value);
    let collapsed = folded.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// Normalised text as a matching key, refusing values over
/// [`MAX_TEXT_CHARS`].
pub(crate) fn text_key(value: &str) -> Normalized<String> {
    if value.chars().count() > MAX_TEXT_CHARS * 4 {
        return Normalized::Unusable(SignalNote::ValueTooLong);
    }
    match normalize_text(value) {
        None => Normalized::Missing,
        Some(text) if text.chars().count() > MAX_TEXT_CHARS => {
            Normalized::Unusable(SignalNote::ValueTooLong)
        }
        Some(text) => Normalized::Value(text),
    }
}

/// A name prepared for fuzzy comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameKey {
    /// The tokens joined by single spaces.
    pub joined: String,
    /// The name's tokens in their original order.
    pub tokens: Vec<String>,
}

/// Characters removed inside a token (`O'Brien` → `obrien`).
const APOSTROPHES: [char; 4] = ['\'', '\u{2019}', '\u{02BC}', '`'];

/// Punctuation that separates name tokens. Letters and combining marks of
/// every script are kept, so names in non-Latin scripts are not split.
fn is_name_separator(c: char) -> bool {
    c.is_whitespace()
        || c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{2010}'
                ..='\u{2015}'
                    | '\u{201C}'
                    | '\u{201D}'
                    | '\u{00B7}'
                    | '\u{2022}'
                    | '\u{3001}'
                    | '\u{3002}'
                    | '\u{FF0C}'
        )
}

/// A name as tokens for fuzzy matching.
///
/// ```
/// use mindgraph_entity_resolution::{Normalized, name_key};
///
/// let Normalized::Value(key) = name_key("M. Ikhwan  Nasution") else { panic!() };
/// assert_eq!(key.tokens, ["m", "ikhwan", "nasution"]);
/// assert_eq!(key.joined, "m ikhwan nasution");
/// ```
#[must_use]
pub fn name_key(value: &str) -> Normalized<NameKey> {
    let text = match text_key(value) {
        Normalized::Value(text) => text,
        Normalized::Missing => return Normalized::Missing,
        Normalized::Unusable(note) => return Normalized::Unusable(note),
    };
    let cleaned: String = text
        .chars()
        .filter(|c| !APOSTROPHES.contains(c))
        .map(|c| if is_name_separator(c) { ' ' } else { c })
        .collect();
    let tokens: Vec<String> = cleaned.split_whitespace().map(str::to_owned).collect();
    if tokens.is_empty() {
        return Normalized::Missing;
    }
    if tokens.len() > MAX_NAME_TOKENS {
        return Normalized::Unusable(SignalNote::ValueTooLong);
    }
    Normalized::Value(NameKey {
        joined: tokens.join(" "),
        tokens,
    })
}

/// An email address prepared for comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailKey {
    /// The local part as written and the lower-cased domain.
    pub exact: String,
    /// The whole address lower-cased: a weaker, case-insensitive key.
    pub folded: String,
}

fn valid_domain(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    domain.chars().count() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.chars().count() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_alphanumeric() || c == '-')
        })
}

/// Normalises an email address conservatively: trimmed, basic structure
/// checked, domain lower-cased, local part preserved.
///
/// ```
/// use mindgraph_entity_resolution::{Normalized, normalize_email};
///
/// let Normalized::Value(key) = normalize_email("  Ikhwan@Example.COM ") else { panic!() };
/// assert_eq!(key.exact, "Ikhwan@example.com");
/// assert_eq!(key.folded, "ikhwan@example.com");
/// ```
#[must_use]
pub fn normalize_email(value: &str) -> Normalized<EmailKey> {
    let value = value.trim();
    if value.is_empty() {
        return Normalized::Missing;
    }
    if value.chars().count() > MAX_EMAIL_CHARS {
        return Normalized::Unusable(SignalNote::ValueTooLong);
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Normalized::Unusable(SignalNote::InvalidValue);
    }
    let Some((local, domain)) = value.rsplit_once('@') else {
        return Normalized::Unusable(SignalNote::InvalidValue);
    };
    if local.is_empty()
        || local.chars().count() > 64
        || local.contains('@')
        || !valid_domain(domain)
    {
        return Normalized::Unusable(SignalNote::InvalidValue);
    }
    let domain = domain.to_lowercase();
    Normalized::Value(EmailKey {
        exact: format!("{local}@{domain}"),
        folded: format!("{}@{domain}", local.to_lowercase()),
    })
}

/// A region for national-format phone numbers, such as `ID` or `US`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Country(phonenumber::country::Id);

impl Country {
    /// Parses an ISO 3166-1 alpha-2 code known to the phone metadata.
    #[must_use]
    pub fn parse(code: &str) -> Option<Self> {
        let code = code.trim();
        if code.len() != 2 || !code.chars().all(|c| c.is_ascii_uppercase()) {
            return None;
        }
        code.parse::<phonenumber::country::Id>().ok().map(Self)
    }
}

/// Normalises a phone number to E.164.
///
/// A number written with a `+country` prefix needs no country. A national
/// number needs `country`; without one it is
/// [`SignalNote::MissingCountry`], never guessed. Numbers the metadata does
/// not consider valid are [`SignalNote::InvalidValue`].
///
/// ```
/// use mindgraph_entity_resolution::{Country, Normalized, normalize_phone};
///
/// let id = Country::parse("ID");
/// assert_eq!(
///     normalize_phone("0812-1234-5678", id),
///     Normalized::Value("+6281212345678".to_owned())
/// );
/// assert_eq!(normalize_phone("+62 812 1234 5678", None), normalize_phone("0812 1234 5678", id));
/// ```
#[must_use]
pub fn normalize_phone(value: &str, country: Option<Country>) -> Normalized<String> {
    let value = value.trim();
    if value.is_empty() {
        return Normalized::Missing;
    }
    if value.chars().count() > MAX_PHONE_CHARS {
        return Normalized::Unusable(SignalNote::ValueTooLong);
    }
    if !value.starts_with('+') && country.is_none() {
        return Normalized::Unusable(SignalNote::MissingCountry);
    }
    match phonenumber::parse(country.map(|c| c.0), value) {
        Ok(number) if phonenumber::is_valid(&number) => {
            Normalized::Value(number.format().mode(phonenumber::Mode::E164).to_string())
        }
        _ => Normalized::Unusable(SignalNote::InvalidValue),
    }
}

/// How two present values relate after normalisation, for the evidence of
/// exact and normalised rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Equality {
    /// Identical as written (after trimming).
    Exact,
    /// Equal only after normalisation.
    Normalized,
    /// Different.
    Different,
}

pub(crate) fn equality(raw_a: &str, raw_b: &str, key_a: &str, key_b: &str) -> Equality {
    if key_a != key_b {
        Equality::Different
    } else if raw_a.trim() == raw_b.trim() {
        Equality::Exact
    } else {
        Equality::Normalized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> NameKey {
        match name_key(value) {
            Normalized::Value(key) => key,
            other => panic!("{value:?} gave {other:?}"),
        }
    }

    #[test]
    fn text_is_unicode_normalised_case_folded_and_collapsed() {
        // NFKC: full-width letters and the ﬁ ligature become plain letters.
        assert_eq!(normalize_text("ＡＬＩＣＥ ﬁsh"), Some("alice fish".into()));
        // Composed and decomposed accents compare equal.
        assert_eq!(normalize_text("Jose\u{301}"), normalize_text("José"));
        // Case folding, not just lower-casing: ß folds to ss.
        assert_eq!(normalize_text("STRASSE"), normalize_text("straße"));
        // Every kind of whitespace run collapses; no-break spaces too.
        assert_eq!(
            normalize_text("  Muhammad \t\n Ikhwan\u{00A0}\u{00A0}Nasution "),
            Some("muhammad ikhwan nasution".into())
        );
    }

    #[test]
    fn blank_text_is_missing() {
        for blank in ["", " ", "\t\n", "\u{00A0}"] {
            assert_eq!(normalize_text(blank), None, "{blank:?}");
            assert_eq!(text_key(blank), Normalized::Missing);
            assert_eq!(name_key(blank), Normalized::Missing);
        }
        // Only punctuation is not a name either.
        assert_eq!(name_key(" . - "), Normalized::Missing);
    }

    #[test]
    fn normalisation_is_idempotent() {
        for sample in [
            "  Muhammad   Ikhwan  ",
            "ＡＬＩＣＥ",
            "STRASSE straße",
            "Ǆemal ǅ",
            "ΣΊΣΥΦΟΣ",
            "İstanbul",
            "ﬀ\u{0345}",
            "Nguyễn Văn A",
        ] {
            let once = normalize_text(sample).unwrap();
            assert_eq!(
                normalize_text(&once).as_deref(),
                Some(once.as_str()),
                "{sample}"
            );
        }
    }

    #[test]
    fn names_split_on_punctuation_and_keep_letters_of_every_script() {
        assert_eq!(
            name("M. Ikhwan K. Nasution").tokens,
            ["m", "ikhwan", "k", "nasution"]
        );
        assert_eq!(name("Tan,Alice").tokens, ["tan", "alice"]);
        assert_eq!(name("O'Brien-Smith").tokens, ["obrien", "smith"]);
        // Devanagari vowel signs are combining marks, not separators.
        assert_eq!(name("राजेश कुमार").tokens.len(), 2);
    }

    #[test]
    fn overlong_values_are_unusable() {
        let long = "a ".repeat(MAX_NAME_TOKENS + 1);
        assert_eq!(
            name_key(&long),
            Normalized::Unusable(SignalNote::ValueTooLong)
        );
        let huge = "x".repeat(MAX_TEXT_CHARS + 1);
        assert_eq!(
            text_key(&huge),
            Normalized::Unusable(SignalNote::ValueTooLong)
        );
    }

    #[test]
    fn email_whitespace_is_trimmed_and_the_domain_lower_cased() {
        let Normalized::Value(key) = normalize_email("\t ikhwan@EXAMPLE.com  ") else {
            panic!()
        };
        assert_eq!(key.exact, "ikhwan@example.com");
        assert_eq!(normalize_email(" "), Normalized::Missing);
    }

    #[test]
    fn email_local_parts_are_not_assumed_identical() {
        let a = normalize_email("Ikhwan@example.com");
        let b = normalize_email("ikhwan@example.com");
        let (Normalized::Value(a), Normalized::Value(b)) = (a, b) else {
            panic!()
        };
        assert_ne!(a.exact, b.exact, "the local part keeps its case");
        assert_eq!(a.folded, b.folded, "only the weaker key ignores case");
        // No provider alias rules: dots and +tags stay significant.
        let dotted = normalize_email("i.khwan+news@gmail.com");
        let plain = normalize_email("ikhwan@gmail.com");
        assert_ne!(
            dotted.value().unwrap().folded,
            plain.value().unwrap().folded
        );
    }

    #[test]
    fn malformed_emails_are_unusable() {
        for bad in [
            "ikhwan",
            "@example.com",
            "ikhwan@",
            "ikhwan@localhost",
            "ikhwan@exa mple.com",
            "ikhwan@example..com",
            "ikhwan@-example.com",
            "a@b@example.com",
        ] {
            assert_eq!(
                normalize_email(bad),
                Normalized::Unusable(SignalNote::InvalidValue),
                "{bad}"
            );
        }
    }

    #[test]
    fn phones_normalise_to_e164_with_a_country() {
        let id = Country::parse("ID");
        let expected = Normalized::Value("+6281212345678".to_owned());
        for input in [
            "+62 812-1234-5678",
            "0812 1234 5678",
            "(0812) 1234-5678",
            "+6281212345678",
        ] {
            assert_eq!(normalize_phone(input, id), expected, "{input}");
        }
    }

    #[test]
    fn national_numbers_need_a_country() {
        assert_eq!(
            normalize_phone("0812 1234 5678", None),
            Normalized::Unusable(SignalNote::MissingCountry)
        );
        // International format works without one.
        assert_eq!(
            normalize_phone("+62 812 1234 5678", None),
            Normalized::Value("+6281212345678".to_owned())
        );
        // The same national digits mean different numbers in other countries.
        assert_ne!(
            normalize_phone("0812 1234 5678", Country::parse("ID")),
            normalize_phone("0812 1234 5678", Country::parse("GB"))
        );
    }

    #[test]
    fn invalid_phones_are_unusable() {
        let id = Country::parse("ID");
        for bad in ["12", "not a phone", "+62 1", "0000000000"] {
            assert_eq!(
                normalize_phone(bad, id),
                Normalized::Unusable(SignalNote::InvalidValue),
                "{bad}"
            );
        }
        assert_eq!(normalize_phone("  ", id), Normalized::Missing);
    }

    #[test]
    fn countries_are_iso_alpha_2() {
        assert!(Country::parse("ID").is_some());
        assert!(Country::parse("US").is_some());
        assert!(Country::parse("id").is_none(), "upper case only");
        assert!(Country::parse("IDN").is_none());
        assert!(Country::parse("ZZ").is_none());
    }
}
