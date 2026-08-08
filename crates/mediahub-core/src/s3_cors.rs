use std::{collections::HashSet, net::Ipv6Addr, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

pub const MAX_S3_CORS_RULES: usize = 100;
pub const MAX_S3_CORS_ID_CHARACTERS: usize = 255;
pub const MAX_S3_CORS_ORIGINS_PER_RULE: usize = 100;
pub const MAX_S3_CORS_HEADERS_PER_RULE: usize = 100;
pub const MAX_S3_CORS_ORIGIN_BYTES: usize = 2_048;
pub const MAX_S3_CORS_HEADER_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S3CorsMethod {
    #[serde(rename = "GET")]
    Get,
    #[serde(rename = "PUT")]
    Put,
    #[serde(rename = "HEAD")]
    Head,
    #[serde(rename = "POST")]
    Post,
    #[serde(rename = "DELETE")]
    Delete,
}

impl S3CorsMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Put => "PUT",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Delete => "DELETE",
        }
    }
}

impl FromStr for S3CorsMethod {
    type Err = S3CorsError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "GET" => Ok(Self::Get),
            "PUT" => Ok(Self::Put),
            "HEAD" => Ok(Self::Head),
            "POST" => Ok(Self::Post),
            "DELETE" => Ok(Self::Delete),
            _ => Err(S3CorsError::InvalidAllowedMethod),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S3CorsRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    allowed_methods: Vec<S3CorsMethod>,
    allowed_origins: Vec<String>,
    allowed_headers: Vec<String>,
    expose_headers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_age_seconds: Option<u32>,
}

impl S3CorsRule {
    pub fn new(
        id: Option<String>,
        allowed_methods: Vec<S3CorsMethod>,
        allowed_origins: Vec<String>,
        allowed_headers: Vec<String>,
        expose_headers: Vec<String>,
        max_age_seconds: Option<u32>,
    ) -> Result<Self, S3CorsError> {
        let rule = Self {
            id,
            allowed_methods,
            allowed_origins,
            allowed_headers,
            expose_headers,
            max_age_seconds,
        };
        rule.validate()?;
        Ok(rule)
    }

    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    #[must_use]
    pub fn allowed_methods(&self) -> &[S3CorsMethod] {
        &self.allowed_methods
    }

    #[must_use]
    pub fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }

    #[must_use]
    pub fn allowed_headers(&self) -> &[String] {
        &self.allowed_headers
    }

    #[must_use]
    pub fn expose_headers(&self) -> &[String] {
        &self.expose_headers
    }

    #[must_use]
    pub const fn max_age_seconds(&self) -> Option<u32> {
        self.max_age_seconds
    }

    pub fn validate(&self) -> Result<(), S3CorsError> {
        if self.id.as_ref().is_some_and(|id| {
            id.is_empty()
                || id.chars().count() > MAX_S3_CORS_ID_CHARACTERS
                || id
                    .chars()
                    .any(|character| character.is_control() || !is_xml_10_character(character))
        }) {
            return Err(S3CorsError::InvalidRuleId);
        }
        validate_methods(&self.allowed_methods)?;
        validate_origins(&self.allowed_origins)?;
        validate_headers(&self.allowed_headers, true)?;
        validate_headers(&self.expose_headers, false)?;
        Ok(())
    }
}

impl<'de> Deserialize<'de> for S3CorsRule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RuleData {
            id: Option<String>,
            allowed_methods: Vec<S3CorsMethod>,
            allowed_origins: Vec<String>,
            #[serde(default)]
            allowed_headers: Vec<String>,
            #[serde(default)]
            expose_headers: Vec<String>,
            max_age_seconds: Option<u32>,
        }

        let data = RuleData::deserialize(deserializer)?;
        Self::new(
            data.id,
            data.allowed_methods,
            data.allowed_origins,
            data.allowed_headers,
            data.expose_headers,
            data.max_age_seconds,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct S3CorsConfiguration {
    rules: Vec<S3CorsRule>,
}

impl S3CorsConfiguration {
    pub fn new(rules: Vec<S3CorsRule>) -> Result<Self, S3CorsError> {
        let configuration = Self { rules };
        configuration.validate()?;
        Ok(configuration)
    }

    #[must_use]
    pub fn rules(&self) -> &[S3CorsRule] {
        &self.rules
    }

    #[must_use]
    pub fn into_rules(self) -> Vec<S3CorsRule> {
        self.rules
    }

    pub fn validate(&self) -> Result<(), S3CorsError> {
        if self.rules.is_empty() {
            return Err(S3CorsError::EmptyConfiguration);
        }
        if self.rules.len() > MAX_S3_CORS_RULES {
            return Err(S3CorsError::TooManyRules);
        }

        let mut ids = HashSet::with_capacity(self.rules.len());
        let mut unique_rules = HashSet::with_capacity(self.rules.len());
        for rule in &self.rules {
            rule.validate()?;
            if !unique_rules.insert(rule) {
                return Err(S3CorsError::DuplicateRule);
            }
            if let Some(id) = rule.id()
                && !ids.insert(id)
            {
                return Err(S3CorsError::DuplicateRuleId);
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for S3CorsConfiguration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ConfigurationData {
            rules: Vec<S3CorsRule>,
        }

        let data = ConfigurationData::deserialize(deserializer)?;
        Self::new(data.rules).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum S3CorsError {
    #[error("a CORS configuration must contain at least one rule")]
    EmptyConfiguration,
    #[error("a CORS configuration cannot contain more than 100 rules")]
    TooManyRules,
    #[error("a CORS configuration contains a duplicate rule")]
    DuplicateRule,
    #[error("a CORS rule ID is invalid")]
    InvalidRuleId,
    #[error("CORS rule IDs must be unique")]
    DuplicateRuleId,
    #[error("a CORS rule must contain at least one allowed method")]
    MissingAllowedMethod,
    #[error("a CORS rule contains a duplicate allowed method")]
    DuplicateAllowedMethod,
    #[error("a CORS allowed method is invalid")]
    InvalidAllowedMethod,
    #[error("a CORS rule must contain at least one allowed origin")]
    MissingAllowedOrigin,
    #[error("a CORS rule cannot contain more than 100 allowed origins")]
    TooManyAllowedOrigins,
    #[error("a CORS allowed origin is invalid")]
    InvalidAllowedOrigin,
    #[error("a CORS rule contains a duplicate allowed origin")]
    DuplicateAllowedOrigin,
    #[error("a CORS rule cannot contain more than 100 allowed headers")]
    TooManyAllowedHeaders,
    #[error("a CORS allowed header pattern is invalid")]
    InvalidAllowedHeader,
    #[error("a CORS rule contains a duplicate allowed header pattern")]
    DuplicateAllowedHeader,
    #[error("a CORS rule cannot contain more than 100 exposed headers")]
    TooManyExposeHeaders,
    #[error("a CORS exposed header is invalid")]
    InvalidExposeHeader,
    #[error("a CORS rule contains a duplicate exposed header")]
    DuplicateExposeHeader,
}

fn validate_methods(methods: &[S3CorsMethod]) -> Result<(), S3CorsError> {
    if methods.is_empty() {
        return Err(S3CorsError::MissingAllowedMethod);
    }
    let mut unique = HashSet::with_capacity(methods.len());
    for method in methods {
        if !unique.insert(*method) {
            return Err(S3CorsError::DuplicateAllowedMethod);
        }
    }
    Ok(())
}

fn validate_origins(origins: &[String]) -> Result<(), S3CorsError> {
    if origins.is_empty() {
        return Err(S3CorsError::MissingAllowedOrigin);
    }
    if origins.len() > MAX_S3_CORS_ORIGINS_PER_RULE {
        return Err(S3CorsError::TooManyAllowedOrigins);
    }
    let mut unique = HashSet::with_capacity(origins.len());
    for origin in origins {
        if !is_valid_origin(origin) {
            return Err(S3CorsError::InvalidAllowedOrigin);
        }
        if !unique.insert(origin.to_ascii_lowercase()) {
            return Err(S3CorsError::DuplicateAllowedOrigin);
        }
    }
    Ok(())
}

fn validate_headers(headers: &[String], wildcard_allowed: bool) -> Result<(), S3CorsError> {
    if headers.len() > MAX_S3_CORS_HEADERS_PER_RULE {
        return Err(if wildcard_allowed {
            S3CorsError::TooManyAllowedHeaders
        } else {
            S3CorsError::TooManyExposeHeaders
        });
    }
    let mut unique = HashSet::with_capacity(headers.len());
    for header in headers {
        let valid = !header.is_empty()
            && header.len() <= MAX_S3_CORS_HEADER_BYTES
            && header.is_ascii()
            && header.bytes().all(is_http_token_byte)
            && if wildcard_allowed {
                header.bytes().filter(|byte| *byte == b'*').count() <= 1
            } else {
                !header.contains('*')
            };
        if !valid {
            return Err(if wildcard_allowed {
                S3CorsError::InvalidAllowedHeader
            } else {
                S3CorsError::InvalidExposeHeader
            });
        }
        if !unique.insert(header.to_ascii_lowercase()) {
            return Err(if wildcard_allowed {
                S3CorsError::DuplicateAllowedHeader
            } else {
                S3CorsError::DuplicateExposeHeader
            });
        }
    }
    Ok(())
}

fn is_valid_origin(origin: &str) -> bool {
    if origin == "*" {
        return true;
    }
    if origin.is_empty()
        || origin.len() > MAX_S3_CORS_ORIGIN_BYTES
        || !origin.is_ascii()
        || origin.bytes().filter(|byte| *byte == b'*').count() > 1
    {
        return false;
    }

    let lowercase = origin.to_ascii_lowercase();
    let authority = if lowercase.starts_with("http://") {
        &origin[7..]
    } else if lowercase.starts_with("https://") {
        &origin[8..]
    } else {
        return false;
    };
    if authority.is_empty()
        || authority
            .bytes()
            .any(|byte| matches!(byte, b'/' | b'?' | b'#' | b'@') || byte.is_ascii_whitespace())
    {
        return false;
    }

    if let Some(ipv6) = authority.strip_prefix('[') {
        let Some((address, suffix)) = ipv6.split_once(']') else {
            return false;
        };
        return !address.contains('*')
            && Ipv6Addr::from_str(address).is_ok()
            && valid_optional_port(suffix);
    }

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            (host, Some(port))
        }
        Some(_) => return false,
        None => (authority, None),
    };
    if port.is_some_and(|port| port.parse::<u16>().is_err())
        || host.is_empty()
        || host.starts_with(['.', '-'])
        || host.ends_with(['.', '-'])
        || host.contains("..")
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'*'))
    {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty() && !label.starts_with('-') && !label.ends_with('-') && label.len() <= 63
    })
}

fn valid_optional_port(suffix: &str) -> bool {
    suffix.is_empty()
        || suffix
            .strip_prefix(':')
            .is_some_and(|port| !port.is_empty() && port.parse::<u16>().is_ok())
}

const fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

const fn is_xml_10_character(character: char) -> bool {
    matches!(
        character,
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rule(id: Option<&str>) -> S3CorsRule {
        S3CorsRule::new(
            id.map(str::to_owned),
            vec![S3CorsMethod::Get, S3CorsMethod::Put],
            vec!["https://*.example.com:8443".to_owned()],
            vec!["x-amz-*".to_owned(), "content-type".to_owned()],
            vec!["etag".to_owned()],
            Some(3_600),
        )
        .expect("valid rule")
    }

    #[test]
    fn valid_configuration_round_trips_through_serde_and_preserves_order() {
        let configuration = S3CorsConfiguration::new(vec![rule(Some("uploads")), rule(None)])
            .expect("valid configuration");
        let json = serde_json::to_value(&configuration).expect("serialize");
        let decoded: S3CorsConfiguration = serde_json::from_value(json).expect("deserialize");

        assert_eq!(decoded, configuration);
        assert_eq!(decoded.rules()[0].allowed_methods()[0], S3CorsMethod::Get);
        assert_eq!(decoded.rules()[0].max_age_seconds(), Some(3_600));
    }

    #[test]
    fn configuration_and_rule_cardinality_are_bounded() {
        assert_eq!(
            S3CorsConfiguration::new(Vec::new()),
            Err(S3CorsError::EmptyConfiguration)
        );
        assert_eq!(
            S3CorsConfiguration::new(vec![rule(None); MAX_S3_CORS_RULES + 1]),
            Err(S3CorsError::TooManyRules)
        );
        assert_eq!(
            S3CorsRule::new(
                None,
                Vec::new(),
                vec!["*".into()],
                Vec::new(),
                Vec::new(),
                None
            ),
            Err(S3CorsError::MissingAllowedMethod)
        );
        assert_eq!(
            S3CorsRule::new(
                None,
                vec![S3CorsMethod::Get],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None
            ),
            Err(S3CorsError::MissingAllowedOrigin)
        );
        assert_eq!(
            S3CorsRule::new(
                None,
                vec![S3CorsMethod::Get],
                vec!["*".into(); MAX_S3_CORS_ORIGINS_PER_RULE + 1],
                Vec::new(),
                Vec::new(),
                None,
            ),
            Err(S3CorsError::TooManyAllowedOrigins)
        );
        assert_eq!(
            S3CorsRule::new(
                None,
                vec![S3CorsMethod::Get],
                vec!["*".into()],
                vec!["x".into(); MAX_S3_CORS_HEADERS_PER_RULE + 1],
                Vec::new(),
                None,
            ),
            Err(S3CorsError::TooManyAllowedHeaders)
        );
        assert_eq!(
            S3CorsRule::new(
                None,
                vec![S3CorsMethod::Get],
                vec!["*".into()],
                Vec::new(),
                vec!["etag".into(); MAX_S3_CORS_HEADERS_PER_RULE + 1],
                None,
            ),
            Err(S3CorsError::TooManyExposeHeaders)
        );
    }

    #[test]
    fn duplicate_values_and_rule_ids_are_rejected() {
        assert_eq!(
            S3CorsRule::new(
                None,
                vec![S3CorsMethod::Get, S3CorsMethod::Get],
                vec!["*".into()],
                Vec::new(),
                Vec::new(),
                None,
            ),
            Err(S3CorsError::DuplicateAllowedMethod)
        );
        assert_eq!(
            S3CorsRule::new(
                None,
                vec![S3CorsMethod::Get],
                vec!["HTTPS://EXAMPLE.COM".into(), "https://example.com".into()],
                Vec::new(),
                Vec::new(),
                None,
            ),
            Err(S3CorsError::DuplicateAllowedOrigin)
        );
        assert_eq!(
            S3CorsRule::new(
                None,
                vec![S3CorsMethod::Get],
                vec!["*".into()],
                vec!["X-Amz-*".into(), "x-amz-*".into()],
                Vec::new(),
                None,
            ),
            Err(S3CorsError::DuplicateAllowedHeader)
        );
        assert_eq!(
            S3CorsConfiguration::new(vec![rule(Some("same")), rule(Some("same"))]),
            Err(S3CorsError::DuplicateRule)
        );
        let mut duplicate_id = rule(Some("same"));
        duplicate_id.allowed_methods = vec![S3CorsMethod::Head];
        assert_eq!(
            S3CorsConfiguration::new(vec![rule(Some("same")), duplicate_id]),
            Err(S3CorsError::DuplicateRuleId)
        );
    }

    #[test]
    fn origins_are_http_origins_with_at_most_one_wildcard() {
        for origin in [
            "*",
            "http://localhost:3000",
            "https://*.example.com",
            "https://[2001:db8::1]:443",
        ] {
            assert!(is_valid_origin(origin), "{origin} should be valid");
        }
        for origin in [
            "",
            "example.com",
            "ftp://example.com",
            "https://*.*.example.com",
            "https://user@example.com",
            "https://example.com/path",
            "https://example.com:70000",
            "https://-example.com",
        ] {
            assert!(!is_valid_origin(origin), "{origin} should be invalid");
        }
    }

    #[test]
    fn allowed_header_patterns_are_tokens_and_exposed_headers_are_concrete() {
        assert!(validate_headers(&["x-amz-*".into(), "content-type".into()], true).is_ok());
        assert_eq!(
            validate_headers(&["x-*-*".into()], true),
            Err(S3CorsError::InvalidAllowedHeader)
        );
        assert_eq!(
            validate_headers(&["content type".into()], true),
            Err(S3CorsError::InvalidAllowedHeader)
        );
        assert_eq!(
            validate_headers(&["x-amz-*".into()], false),
            Err(S3CorsError::InvalidExposeHeader)
        );
        assert_eq!(
            validate_headers(&["x".repeat(MAX_S3_CORS_HEADER_BYTES + 1)], true),
            Err(S3CorsError::InvalidAllowedHeader)
        );
        assert_eq!(
            validate_headers(&["x".repeat(MAX_S3_CORS_HEADER_BYTES + 1)], false),
            Err(S3CorsError::InvalidExposeHeader)
        );
        let oversized_origin = format!(
            "https://{}",
            "a.".repeat((MAX_S3_CORS_ORIGIN_BYTES / 2) + 1)
        );
        assert!(!is_valid_origin(&oversized_origin));
    }

    #[test]
    fn ids_are_non_empty_unique_xml_text_with_a_255_character_limit() {
        for id in ["", &"a".repeat(MAX_S3_CORS_ID_CHARACTERS + 1), "bad\u{1}"] {
            assert_eq!(
                S3CorsRule::new(
                    Some(id.to_owned()),
                    vec![S3CorsMethod::Get],
                    vec!["*".into()],
                    Vec::new(),
                    Vec::new(),
                    None,
                ),
                Err(S3CorsError::InvalidRuleId)
            );
        }
        assert!(
            S3CorsRule::new(
                Some("界".repeat(MAX_S3_CORS_ID_CHARACTERS)),
                vec![S3CorsMethod::Get],
                vec!["*".into()],
                Vec::new(),
                Vec::new(),
                Some(0),
            )
            .is_ok()
        );
    }

    #[test]
    fn deserialization_cannot_bypass_domain_validation() {
        let invalid_method = json!({
            "rules": [{
                "allowed_methods": ["OPTIONS"],
                "allowed_origins": ["*"]
            }]
        });
        assert!(serde_json::from_value::<S3CorsConfiguration>(invalid_method).is_err());

        let unknown_field = json!({
            "rules": [{
                "allowed_methods": ["GET"],
                "allowed_origins": ["*"],
                "passthrough": true
            }]
        });
        assert!(serde_json::from_value::<S3CorsConfiguration>(unknown_field).is_err());
    }
}
