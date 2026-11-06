//! Matching rules and their validation against the active ontology.
//!
//! Every rule names one property of the scanned entity type and how to
//! compare it. Rules are explicit: no property is treated as an identifier
//! just because it is called `id`. Entities are only compared with entities
//! of the same type.

use std::collections::HashSet;

use mindgraph_domain::{EntityType, PropertyKey, PropertyType, TypeName};
use serde::{Deserialize, Serialize};

use crate::RuleError;
use crate::error::echo;
use crate::normalize::Country;

/// Most rules in one scan.
pub const MAX_RULES: usize = 10;
/// The fuzzy name threshold used when none is given.
pub const DEFAULT_FUZZY_THRESHOLD: f64 = 0.90;
/// Smallest accepted fuzzy threshold.
pub const MIN_FUZZY_THRESHOLD: f64 = 0.5;
/// Largest accepted fuzzy threshold.
pub const MAX_FUZZY_THRESHOLD: f64 = 1.0;

/// How a rule compares a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// Equal configured identifiers (trimmed, case-sensitive): strong
    /// evidence, still reviewed by a person in M9.
    ExactIdentifier,
    /// Equal email addresses after conservative normalisation.
    NormalizedEmail,
    /// Equal phone numbers in E.164 form.
    NormalizedPhone,
    /// Equal text after Unicode normalisation, case folding and whitespace
    /// collapsing.
    NormalizedString,
    /// Similar names (Jaro-Winkler, token-aware) at or above a threshold:
    /// weak evidence.
    FuzzyName,
}

impl RuleKind {
    /// Every kind, strongest evidence first.
    pub const ALL: [Self; 5] = [
        Self::ExactIdentifier,
        Self::NormalizedEmail,
        Self::NormalizedPhone,
        Self::NormalizedString,
        Self::FuzzyName,
    ];

    /// The stable name of this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactIdentifier => "exact_identifier",
            Self::NormalizedEmail => "normalized_email",
            Self::NormalizedPhone => "normalized_phone",
            Self::NormalizedString => "normalized_string",
            Self::FuzzyName => "fuzzy_name",
        }
    }

    /// Parses a kind by its stable name.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value.trim())
    }

    /// Whether this kind can compare a property of `property_type`.
    #[must_use]
    pub const fn accepts(self, property_type: PropertyType) -> bool {
        match self {
            Self::ExactIdentifier => {
                matches!(property_type, PropertyType::String | PropertyType::Integer)
            }
            // Integer phones are accepted so their values can be reported as
            // unusable (leading zeros may be lost) rather than rejected.
            Self::NormalizedPhone => {
                matches!(property_type, PropertyType::String | PropertyType::Integer)
            }
            Self::NormalizedEmail | Self::NormalizedString | Self::FuzzyName => {
                matches!(property_type, PropertyType::String)
            }
        }
    }
}

/// A rule as a client sends it, before validation.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct RuleSpec {
    /// The property to compare.
    pub property: String,
    /// The rule kind's stable name, such as `normalized_email`.
    pub kind: String,
    /// `fuzzy_name` only: the similarity threshold (default 0.90).
    #[serde(default)]
    pub threshold: Option<f64>,
    /// `normalized_phone` only: the ISO 3166-1 alpha-2 region of numbers
    /// written without `+country`.
    #[serde(default)]
    pub country: Option<String>,
    /// `normalized_email` only: whether addresses that differ only in the
    /// letter case of their local part count as a (weaker) match. Default
    /// `true`.
    #[serde(default)]
    pub case_insensitive_local_part: Option<bool>,
}

/// A validated rule. Its snapshot is stored with every scan, so later rule
/// changes never rewrite historical evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchRule {
    /// The compared property.
    pub property: PropertyKey,
    /// The rule kind.
    pub kind: RuleKind,
    /// The property's type in the ontology the scan used.
    pub property_type: PropertyType,
    /// `fuzzy_name`: the similarity threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// `normalized_phone`: the default region, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// `normalized_email`: whether case-only local-part differences match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_insensitive_local_part: Option<bool>,
}

impl MatchRule {
    /// The fuzzy threshold (the default when unset).
    #[must_use]
    pub fn fuzzy_threshold(&self) -> f64 {
        self.threshold.unwrap_or(DEFAULT_FUZZY_THRESHOLD)
    }

    /// The phone rule's default region.
    #[must_use]
    pub fn country(&self) -> Option<Country> {
        self.country.as_deref().and_then(Country::parse)
    }

    /// Whether case-only local-part differences count as an email match.
    #[must_use]
    pub fn email_case_insensitive(&self) -> bool {
        self.case_insensitive_local_part.unwrap_or(true)
    }
}

/// The validated rules of one scan, all for one entity type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSet {
    /// The scanned entity type.
    pub entity_type: TypeName,
    /// The rules, in the order given.
    pub rules: Vec<MatchRule>,
}

fn reject_option<T>(
    value: Option<&T>,
    option: &'static str,
    kind: RuleKind,
) -> Result<(), RuleError> {
    if value.is_some() {
        Err(RuleError::OptionNotAllowed {
            option,
            kind: kind.as_str(),
        })
    } else {
        Ok(())
    }
}

impl RuleSet {
    /// Validates `specs` against `entity_type` from the active ontology:
    /// known kinds, defined properties with a comparable type, options only
    /// where they apply, a threshold in range, a known country, no duplicate
    /// `(property, kind)` and at most [`MAX_RULES`] rules.
    ///
    /// # Errors
    ///
    /// The first [`RuleError`] found.
    pub fn validate(entity_type: &EntityType, specs: &[RuleSpec]) -> Result<Self, RuleError> {
        if specs.is_empty() {
            return Err(RuleError::NoRules);
        }
        if specs.len() > MAX_RULES {
            return Err(RuleError::TooManyRules { max: MAX_RULES });
        }
        let mut seen = HashSet::new();
        let mut rules = Vec::with_capacity(specs.len());
        for spec in specs {
            let kind = RuleKind::parse(&spec.kind)
                .ok_or_else(|| RuleError::UnknownKind(echo(&spec.kind)))?;
            let property = PropertyKey::new(spec.property.trim())
                .map_err(|_| RuleError::InvalidPropertyName(echo(&spec.property)))?;
            let definition = entity_type
                .properties_schema()
                .get(property.as_str())
                .ok_or_else(|| RuleError::UnknownProperty {
                    property: property.to_string(),
                    entity_type: entity_type.name().to_string(),
                })?;
            if !kind.accepts(definition.property_type) {
                return Err(RuleError::UnsupportedPropertyType {
                    property: property.to_string(),
                    kind: kind.as_str(),
                    property_type: definition.property_type,
                });
            }
            if !seen.insert((property.clone(), kind)) {
                return Err(RuleError::DuplicateRule {
                    property: property.to_string(),
                    kind: kind.as_str(),
                });
            }
            let mut rule = MatchRule {
                property,
                kind,
                property_type: definition.property_type,
                threshold: None,
                country: None,
                case_insensitive_local_part: None,
            };
            match kind {
                RuleKind::FuzzyName => {
                    let threshold = spec.threshold.unwrap_or(DEFAULT_FUZZY_THRESHOLD);
                    if !(MIN_FUZZY_THRESHOLD..=MAX_FUZZY_THRESHOLD).contains(&threshold) {
                        return Err(RuleError::InvalidThreshold {
                            min: MIN_FUZZY_THRESHOLD,
                            max: MAX_FUZZY_THRESHOLD,
                        });
                    }
                    rule.threshold = Some(threshold);
                }
                _ => reject_option(spec.threshold.as_ref(), "threshold", kind)?,
            }
            match kind {
                RuleKind::NormalizedPhone => {
                    if let Some(raw) = spec.country.as_deref().map(str::trim)
                        && !raw.is_empty()
                    {
                        let code = raw.to_ascii_uppercase();
                        if Country::parse(&code).is_none() {
                            return Err(RuleError::InvalidCountry(echo(raw)));
                        }
                        rule.country = Some(code);
                    }
                }
                _ => reject_option(spec.country.as_ref(), "country", kind)?,
            }
            match kind {
                RuleKind::NormalizedEmail => {
                    rule.case_insensitive_local_part =
                        Some(spec.case_insensitive_local_part.unwrap_or(true));
                }
                _ => reject_option(
                    spec.case_insensitive_local_part.as_ref(),
                    "case_insensitive_local_part",
                    kind,
                )?,
            }
            rules.push(rule);
        }
        Ok(Self {
            entity_type: entity_type.name().clone(),
            rules,
        })
    }

    /// `kind:property` labels of the rules, for provenance.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.rules
            .iter()
            .map(|rule| format!("{}:{}", rule.kind.as_str(), rule.property))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use mindgraph_domain::{EntityTypeId, GraphId, PropertiesSchema, PropertyDefinition};

    use super::*;

    fn customer() -> EntityType {
        let property = |name: &str, property_type| {
            PropertyDefinition::optional(PropertyKey::new(name).unwrap(), property_type)
        };
        EntityType::new(
            EntityTypeId::new(),
            GraphId::new(),
            TypeName::new("Customer").unwrap(),
            PropertiesSchema::new(vec![
                property("customer_id", PropertyType::String),
                property("name", PropertyType::String),
                property("email", PropertyType::String),
                property("phone", PropertyType::String),
                property("legacy_phone", PropertyType::Integer),
                property("age", PropertyType::Integer),
                property("tags", PropertyType::StringArray),
            ])
            .unwrap(),
        )
    }

    fn spec(property: &str, kind: &str) -> RuleSpec {
        RuleSpec {
            property: property.into(),
            kind: kind.into(),
            ..RuleSpec::default()
        }
    }

    #[test]
    fn valid_rules_get_defaults() {
        let rules = RuleSet::validate(
            &customer(),
            &[
                spec("customer_id", "exact_identifier"),
                spec("email", "normalized_email"),
                RuleSpec {
                    country: Some("id".into()),
                    ..spec("phone", "normalized_phone")
                },
                spec("name", "fuzzy_name"),
            ],
        )
        .unwrap();
        assert_eq!(rules.entity_type.as_str(), "Customer");
        assert_eq!(rules.rules[1].case_insensitive_local_part, Some(true));
        assert_eq!(rules.rules[2].country.as_deref(), Some("ID"));
        assert_eq!(rules.rules[3].threshold, Some(DEFAULT_FUZZY_THRESHOLD));
        assert_eq!(
            rules.labels(),
            [
                "exact_identifier:customer_id",
                "normalized_email:email",
                "normalized_phone:phone",
                "fuzzy_name:name"
            ]
        );
    }

    #[test]
    fn invalid_rules_are_rejected_with_clear_errors() {
        let customer = customer();
        let check = |specs: &[RuleSpec]| RuleSet::validate(&customer, specs).unwrap_err();
        assert_eq!(check(&[]), RuleError::NoRules);
        assert!(matches!(
            check(&[spec("name", "soundex")]),
            RuleError::UnknownKind(_)
        ));
        assert!(matches!(
            check(&[spec("unknown", "fuzzy_name")]),
            RuleError::UnknownProperty { .. }
        ));
        assert!(matches!(
            check(&[spec("bad name", "fuzzy_name")]),
            RuleError::InvalidPropertyName(_)
        ));
        assert!(matches!(
            check(&[spec("age", "fuzzy_name")]),
            RuleError::UnsupportedPropertyType { .. }
        ));
        assert!(matches!(
            check(&[spec("tags", "exact_identifier")]),
            RuleError::UnsupportedPropertyType { .. }
        ));
        for threshold in [0.0, 0.49, 1.01, f64::NAN] {
            assert!(matches!(
                check(&[RuleSpec {
                    threshold: Some(threshold),
                    ..spec("name", "fuzzy_name")
                }]),
                RuleError::InvalidThreshold { .. }
            ));
        }
        assert!(matches!(
            check(&[RuleSpec {
                threshold: Some(0.9),
                ..spec("email", "normalized_email")
            }]),
            RuleError::OptionNotAllowed { .. }
        ));
        assert!(matches!(
            check(&[RuleSpec {
                country: Some("XX".into()),
                ..spec("phone", "normalized_phone")
            }]),
            RuleError::InvalidCountry(_)
        ));
        assert!(matches!(
            check(&[spec("name", "fuzzy_name"), spec("name", "fuzzy_name")]),
            RuleError::DuplicateRule { .. }
        ));
        let many: Vec<RuleSpec> = (0..=MAX_RULES)
            .map(|_| spec("name", "fuzzy_name"))
            .collect();
        assert_eq!(check(&many), RuleError::TooManyRules { max: MAX_RULES });
    }

    #[test]
    fn integer_phones_are_accepted_for_reporting() {
        let rules =
            RuleSet::validate(&customer(), &[spec("legacy_phone", "normalized_phone")]).unwrap();
        assert_eq!(rules.rules[0].property_type, PropertyType::Integer);
    }

    #[test]
    fn rule_snapshots_round_trip_as_json() {
        let rules = RuleSet::validate(
            &customer(),
            &[
                spec("email", "normalized_email"),
                spec("name", "fuzzy_name"),
            ],
        )
        .unwrap();
        let json = serde_json::to_value(&rules).unwrap();
        assert_eq!(json["rules"][1]["kind"], "fuzzy_name");
        assert_eq!(json["rules"][1]["threshold"], 0.9);
        let back: RuleSet = serde_json::from_value(json).unwrap();
        assert_eq!(back, rules);
    }
}
