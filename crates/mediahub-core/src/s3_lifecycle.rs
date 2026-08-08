use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::S3ModelError;

pub const MAX_S3_LIFECYCLE_RULES: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S3LifecycleRuleStatus {
    Enabled,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum S3LifecycleFilter {
    Empty,
    Prefix(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum S3Expiration {
    Days(u32),
    Date(OffsetDateTime),
    ExpiredObjectDeleteMarker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3NoncurrentVersionExpiration {
    pub noncurrent_days: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3AbortIncompleteMultipartUpload {
    pub days_after_initiation: u32,
}

/// First-phase lifecycle rule. Transitions, tag filters and size filters are
/// deliberately absent and rejected by `from_normalized_json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3LifecycleRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub status: S3LifecycleRuleStatus,
    pub filter: S3LifecycleFilter,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration: Option<S3Expiration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noncurrent_version_expiration: Option<S3NoncurrentVersionExpiration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abort_incomplete_multipart_upload: Option<S3AbortIncompleteMultipartUpload>,
}

impl S3LifecycleRule {
    pub fn validate(&self) -> Result<(), S3ModelError> {
        if self.id.as_ref().is_some_and(|id| {
            id.is_empty() || id.len() > 255 || id.bytes().any(|byte| byte.is_ascii_control())
        }) {
            return Err(S3ModelError::InvalidLifecycleConfiguration);
        }
        match &self.filter {
            S3LifecycleFilter::Empty => {}
            S3LifecycleFilter::Prefix(prefix) => {
                if prefix.len() > 1_024 || prefix.contains('\0') {
                    return Err(S3ModelError::InvalidLifecycleConfiguration);
                }
            }
        }
        if matches!(self.expiration, Some(S3Expiration::Days(0)))
            || self
                .noncurrent_version_expiration
                .is_some_and(|action| action.noncurrent_days == 0)
            || self
                .abort_incomplete_multipart_upload
                .is_some_and(|action| action.days_after_initiation == 0)
        {
            return Err(S3ModelError::InvalidLifecycleConfiguration);
        }
        if self.expiration.is_none()
            && self.noncurrent_version_expiration.is_none()
            && self.abort_incomplete_multipart_upload.is_none()
        {
            return Err(S3ModelError::InvalidLifecycleConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3LifecycleConfiguration {
    pub rules: Vec<S3LifecycleRule>,
}

impl S3LifecycleConfiguration {
    pub fn new(rules: Vec<S3LifecycleRule>) -> Result<Self, S3ModelError> {
        let configuration = Self { rules };
        configuration.validate()?;
        Ok(configuration)
    }

    /// Validates the normalized JSON boundary and reports unknown rule fields
    /// as unsupported actions instead of silently retaining opaque behavior.
    pub fn from_normalized_json(value: Value) -> Result<Self, S3ModelError> {
        let object = value
            .as_object()
            .ok_or(S3ModelError::InvalidLifecycleConfiguration)?;
        if object.keys().any(|key| key != "rules") {
            return Err(S3ModelError::InvalidLifecycleConfiguration);
        }
        let rules = object
            .get("rules")
            .and_then(Value::as_array)
            .ok_or(S3ModelError::InvalidLifecycleConfiguration)?;
        const ALLOWED_RULE_FIELDS: &[&str] = &[
            "id",
            "status",
            "filter",
            "expiration",
            "noncurrent_version_expiration",
            "abort_incomplete_multipart_upload",
        ];
        for rule in rules {
            let rule = rule
                .as_object()
                .ok_or(S3ModelError::InvalidLifecycleConfiguration)?;
            if let Some(unsupported) = rule
                .keys()
                .find(|key| !ALLOWED_RULE_FIELDS.contains(&key.as_str()))
            {
                return Err(S3ModelError::UnsupportedLifecycleAction(
                    unsupported.clone(),
                ));
            }
        }
        let configuration: Self = serde_json::from_value(value)
            .map_err(|_| S3ModelError::InvalidLifecycleConfiguration)?;
        configuration.validate()?;
        Ok(configuration)
    }

    pub fn validate(&self) -> Result<(), S3ModelError> {
        if self.rules.is_empty() || self.rules.len() > MAX_S3_LIFECYCLE_RULES {
            return Err(S3ModelError::InvalidLifecycleConfiguration);
        }
        let mut ids = HashSet::with_capacity(self.rules.len());
        for rule in &self.rules {
            rule.validate()?;
            if let Some(id) = rule.id.as_deref() {
                if !ids.insert(id) {
                    return Err(S3ModelError::InvalidLifecycleConfiguration);
                }
            }
        }
        Ok(())
    }

    pub fn to_normalized_json(&self) -> Result<Value, S3ModelError> {
        self.validate()?;
        serde_json::to_value(self).map_err(|_| S3ModelError::InvalidLifecycleConfiguration)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rule() -> S3LifecycleRule {
        S3LifecycleRule {
            id: Some("expire-temp".into()),
            status: S3LifecycleRuleStatus::Enabled,
            filter: S3LifecycleFilter::Prefix("tmp/".into()),
            expiration: Some(S3Expiration::Days(30)),
            noncurrent_version_expiration: None,
            abort_incomplete_multipart_upload: Some(S3AbortIncompleteMultipartUpload {
                days_after_initiation: 7,
            }),
        }
    }

    #[test]
    fn first_phase_actions_round_trip() {
        let configuration = S3LifecycleConfiguration::new(vec![rule()]).expect("configuration");
        let json = configuration.to_normalized_json().expect("json");
        assert_eq!(
            S3LifecycleConfiguration::from_normalized_json(json),
            Ok(configuration)
        );
    }

    #[test]
    fn unsupported_transition_is_rejected_explicitly() {
        let mut value = serde_json::to_value(S3LifecycleConfiguration {
            rules: vec![rule()],
        })
        .expect("json");
        value["rules"][0]["transition"] = json!({"days": 1, "storage_class": "GLACIER"});
        assert_eq!(
            S3LifecycleConfiguration::from_normalized_json(value),
            Err(S3ModelError::UnsupportedLifecycleAction(
                "transition".into()
            ))
        );
    }

    #[test]
    fn zero_day_action_is_rejected() {
        let mut invalid = rule();
        invalid.expiration = Some(S3Expiration::Days(0));
        assert_eq!(
            S3LifecycleConfiguration::new(vec![invalid]),
            Err(S3ModelError::InvalidLifecycleConfiguration)
        );
    }
}
