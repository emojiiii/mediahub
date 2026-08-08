use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_S3_OBJECT_TAGS: usize = 10;
pub const MAX_S3_BUCKET_TAGS: usize = 50;
pub const MAX_S3_OBJECT_TAG_KEY_CHARS: usize = 128;
pub const MAX_S3_OBJECT_TAG_VALUE_CHARS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3ObjectTag {
    key: String,
    value: String,
}

impl S3ObjectTag {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Result<Self, S3TaggingError> {
        let tag = Self {
            key: key.into(),
            value: value.into(),
        };
        tag.validate()?;
        Ok(tag)
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    fn validate(&self) -> Result<(), S3TaggingError> {
        validate_tag_text(
            &self.key,
            1,
            MAX_S3_OBJECT_TAG_KEY_CHARS,
            S3TaggingError::InvalidKey,
        )?;
        validate_tag_text(
            &self.value,
            0,
            MAX_S3_OBJECT_TAG_VALUE_CHARS,
            S3TaggingError::InvalidValue,
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct S3ObjectTagSet(Vec<S3ObjectTag>);

impl S3ObjectTagSet {
    pub fn new(tags: Vec<S3ObjectTag>) -> Result<Self, S3TaggingError> {
        validate_tag_set(&tags, MAX_S3_OBJECT_TAGS)?;
        Ok(Self(tags))
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &S3ObjectTag> {
        self.0.iter()
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<S3ObjectTag> {
        self.0
    }

    pub fn validate(&self) -> Result<(), S3TaggingError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

/// Bucket tags use the same AWS key/value character rules as object tags but
/// have a distinct 50-tag resource limit. Keeping a separate set type avoids
/// accidentally applying the object protocol's 10-tag cap to a bucket.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct S3BucketTagSet(Vec<S3ObjectTag>);

impl S3BucketTagSet {
    pub fn new(tags: Vec<S3ObjectTag>) -> Result<Self, S3TaggingError> {
        validate_tag_set(&tags, MAX_S3_BUCKET_TAGS)?;
        Ok(Self(tags))
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &S3ObjectTag> {
        self.0.iter()
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<S3ObjectTag> {
        self.0
    }

    pub fn validate(&self) -> Result<(), S3TaggingError> {
        Self::new(self.0.clone()).map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum S3TaggingError {
    #[error("the tag set exceeds the resource limit")]
    TooManyTags,
    #[error("a tag key is invalid")]
    InvalidKey,
    #[error("a tag value is invalid")]
    InvalidValue,
    #[error("tag keys must be unique")]
    DuplicateKey,
}

fn validate_tag_set(tags: &[S3ObjectTag], maximum_tags: usize) -> Result<(), S3TaggingError> {
    if tags.len() > maximum_tags {
        return Err(S3TaggingError::TooManyTags);
    }
    let mut keys = HashSet::with_capacity(tags.len());
    for tag in tags {
        tag.validate()?;
        if !keys.insert(tag.key()) {
            return Err(S3TaggingError::DuplicateKey);
        }
    }
    Ok(())
}

fn validate_tag_text(
    value: &str,
    minimum_chars: usize,
    maximum_chars: usize,
    error: S3TaggingError,
) -> Result<(), S3TaggingError> {
    let count = value.chars().count();
    if !(minimum_chars..=maximum_chars).contains(&count)
        || !value.chars().all(is_valid_tag_character)
    {
        return Err(error);
    }
    Ok(())
}

fn is_valid_tag_character(character: char) -> bool {
    character.is_alphabetic()
        || character.is_numeric()
        || (character.is_whitespace() && !character.is_control())
        || matches!(character, '_' | '.' | ':' | '/' | '=' | '+' | '-' | '@')
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_S3_BUCKET_TAGS, MAX_S3_OBJECT_TAGS, S3BucketTagSet, S3ObjectTag, S3ObjectTagSet,
        S3TaggingError,
    };

    #[test]
    fn accepts_standard_unicode_tag_character_set_and_preserves_order() {
        let tags = S3ObjectTagSet::new(vec![
            S3ObjectTag::new("项目:颜色", "深 蓝/+_-.=@").expect("valid tag"),
            S3ObjectTag::new("stage", "prod").expect("valid tag"),
        ])
        .expect("valid tag set");

        assert_eq!(tags.len(), 2);
        assert_eq!(tags.iter().next().expect("first").key(), "项目:颜色");
    }

    #[test]
    fn rejects_limits_invalid_characters_and_duplicate_keys() {
        assert_eq!(
            S3ObjectTag::new("", "value"),
            Err(S3TaggingError::InvalidKey)
        );
        assert_eq!(
            S3ObjectTag::new("key", "bad&value"),
            Err(S3TaggingError::InvalidValue)
        );
        assert_eq!(
            S3ObjectTag::new("line\nbreak", "value"),
            Err(S3TaggingError::InvalidKey)
        );
        assert_eq!(
            S3ObjectTag::new("key", "tab\tvalue"),
            Err(S3TaggingError::InvalidValue)
        );
        assert_eq!(
            S3ObjectTagSet::new(vec![
                S3ObjectTag::new("same", "one").expect("valid"),
                S3ObjectTag::new("same", "two").expect("valid"),
            ]),
            Err(S3TaggingError::DuplicateKey)
        );
        let too_many = (0..=MAX_S3_OBJECT_TAGS)
            .map(|index| S3ObjectTag::new(format!("key{index}"), "value").expect("valid"))
            .collect();
        assert_eq!(
            S3ObjectTagSet::new(too_many),
            Err(S3TaggingError::TooManyTags)
        );
    }

    #[test]
    fn lengths_are_counted_as_unicode_characters_not_bytes() {
        assert!(S3ObjectTag::new("界".repeat(128), "值".repeat(256)).is_ok());
        assert_eq!(
            S3ObjectTag::new("界".repeat(129), "value"),
            Err(S3TaggingError::InvalidKey)
        );
        assert_eq!(
            S3ObjectTag::new("key", "值".repeat(257)),
            Err(S3TaggingError::InvalidValue)
        );
    }

    #[test]
    fn bucket_and_object_tag_sets_keep_distinct_aws_limits() {
        let tags = (0..MAX_S3_BUCKET_TAGS)
            .map(|index| S3ObjectTag::new(format!("key{index}"), "value").expect("valid"))
            .collect::<Vec<_>>();
        assert_eq!(
            S3BucketTagSet::new(tags.clone())
                .expect("bucket tags")
                .len(),
            50
        );
        assert_eq!(S3ObjectTagSet::new(tags), Err(S3TaggingError::TooManyTags));
        let too_many = (0..=MAX_S3_BUCKET_TAGS)
            .map(|index| S3ObjectTag::new(format!("key{index}"), "value").expect("valid"))
            .collect();
        assert_eq!(
            S3BucketTagSet::new(too_many),
            Err(S3TaggingError::TooManyTags)
        );
    }
}
