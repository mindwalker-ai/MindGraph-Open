//! Errors of rule validation and identity group planning.
//!
//! Messages name properties, rule kinds and limits. They never contain
//! property values, so they are safe to show to clients and to log.

use mindgraph_domain::PropertyType;

/// A matching rule cannot be used with the selected entity type.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RuleError {
    /// No rule was given.
    #[error("at least one matching rule is required")]
    NoRules,

    /// More rules than [`MAX_RULES`](crate::MAX_RULES).
    #[error("at most {max} matching rules are allowed")]
    TooManyRules {
        /// The limit.
        max: usize,
    },

    /// The rule kind is not one of the supported kinds.
    #[error(
        "unknown rule kind `{0}`; use exact_identifier, normalized_email, normalized_phone, normalized_string or fuzzy_name"
    )]
    UnknownKind(String),

    /// The property name is not a valid machine name.
    #[error("`{0}` is not a valid property name")]
    InvalidPropertyName(String),

    /// The entity type does not define the property.
    #[error("property `{property}` is not defined for `{entity_type}`")]
    UnknownProperty {
        /// The property.
        property: String,
        /// The entity type.
        entity_type: String,
    },

    /// The property's type cannot be compared by this rule kind.
    #[error("rule `{kind}` cannot compare property `{property}` of type {property_type}")]
    UnsupportedPropertyType {
        /// The property.
        property: String,
        /// The rule kind.
        kind: &'static str,
        /// The property's type in the ontology.
        property_type: PropertyType,
    },

    /// The fuzzy threshold is outside its range.
    #[error("threshold must be a number between {min} and {max}")]
    InvalidThreshold {
        /// Smallest accepted threshold.
        min: f64,
        /// Largest accepted threshold.
        max: f64,
    },

    /// An option was given to a rule kind that does not use it.
    #[error("option `{option}` is not allowed for rule `{kind}`")]
    OptionNotAllowed {
        /// The option name.
        option: &'static str,
        /// The rule kind.
        kind: &'static str,
    },

    /// The phone rule's country is not an ISO 3166-1 alpha-2 region known to
    /// the phone number metadata.
    #[error("`{0}` is not a supported ISO 3166-1 alpha-2 country code")]
    InvalidCountry(String),

    /// The same property is configured twice with the same rule kind.
    #[error("rule `{kind}` is configured twice for property `{property}`")]
    DuplicateRule {
        /// The property.
        property: String,
        /// The rule kind.
        kind: &'static str,
    },
}

/// Two identity groups cannot be consolidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    /// An entity cannot be merged with itself.
    #[error("an entity cannot be linked with itself")]
    SelfPair,

    /// The chosen canonical entity is not a member of the resulting group.
    #[error("the canonical entity must be a member of the approved identity group")]
    CanonicalOutsideGroup,

    /// The resulting group would exceed the configured size.
    #[error("an identity group may have at most {max} members")]
    GroupTooLarge {
        /// The limit.
        max: usize,
    },

    /// A supplied group does not contain the entity it was loaded for.
    #[error("identity group data is inconsistent")]
    InconsistentGroup,
}

/// Shortens user-supplied text echoed in an error message.
pub(crate) fn echo(text: &str) -> String {
    const MAX: usize = 64;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX {
        trimmed.to_owned()
    } else {
        let mut short: String = trimmed.chars().take(MAX).collect();
        short.push('…');
        short
    }
}
