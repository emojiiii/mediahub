// Pure S3 Bucket Tagging protocol primitives.
//
// This file intentionally contains no authorization or persistence. It is designed to be
// included by `s3_http.rs` once the Bucket Tagging control-plane repository is available.

#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum S3BucketTaggingError {
    #[error("a bucket cannot have more than 50 tags")]
    TooManyTags,
    #[error("a bucket tag key is invalid")]
    InvalidKey,
    #[error("a bucket tag value is invalid")]
    InvalidValue,
    #[error("bucket tag keys must be unique")]
    DuplicateKey,
}

#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub(super) enum S3BucketTaggingXmlError {
    #[error("the bucket tagging XML request body exceeds the supported limit")]
    InputTooLarge,
    #[error("the bucket tagging XML request body is malformed")]
    MalformedXml,
    #[error("the bucket tag set is invalid: {0}")]
    InvalidTag(S3BucketTaggingError),
}

fn map_s3_bucket_tag_validation_error(
    error: mediahub_core::S3TaggingError,
) -> S3BucketTaggingError {
    match error {
        mediahub_core::S3TaggingError::TooManyTags => S3BucketTaggingError::TooManyTags,
        mediahub_core::S3TaggingError::InvalidKey => S3BucketTaggingError::InvalidKey,
        mediahub_core::S3TaggingError::InvalidValue => S3BucketTaggingError::InvalidValue,
        mediahub_core::S3TaggingError::DuplicateKey => S3BucketTaggingError::DuplicateKey,
    }
}

/// Returns `true` only for an unambiguous `?tagging` Bucket subresource query.
///
/// SigV4 query authentication fields and the AWS SDK `x-id` field may accompany the
/// subresource. Any non-empty, duplicate, or combined S3 subresource is rejected before a
/// caller dispatches GET, PUT, or DELETE.
pub(super) fn classify_s3_bucket_tagging(uri: &Uri, request_id: &str) -> Result<bool, S3ApiError> {
    let mut tagging_seen = false;
    let mut unexpected_parameter = false;
    for (name, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        match name.as_ref() {
            "tagging" if !tagging_seen && value.is_empty() => tagging_seen = true,
            "tagging" => {
                return Err(S3ApiError::invalid_argument(
                    "The tagging subresource must occur once with an empty value.",
                    uri.path(),
                    request_id,
                ));
            }
            name if name.starts_with("X-Amz-") || name == "x-id" => {}
            _ => unexpected_parameter = true,
        }
    }
    if tagging_seen && unexpected_parameter {
        return Err(S3ApiError::invalid_request(
            "Bucket Tagging cannot be combined with another subresource or listing parameter.",
            uri.path(),
            request_id,
        ));
    }
    Ok(tagging_seen)
}

/// Verifies the mandatory Content-MD5 and decodes the replacement tag set for PUT.
///
/// An empty `TagSet` is deliberately returned as an empty value. Silo's Bucket Tagging parser
/// accepts it as a valid replacement configuration; only DELETE should remove persistence.
pub(super) fn parse_s3_bucket_tagging_put(
    headers: &HeaderMap,
    content: &[u8],
    resource: &str,
    request_id: &str,
) -> Result<mediahub_core::S3BucketTagSet, S3ApiError> {
    validate_content_md5(
        single_s3_content_md5(headers, resource, request_id)?,
        content,
    )
    .map_err(|error| match error {
        super::s3_xml::ContentMd5Error::InvalidDigest => {
            S3ApiError::invalid_digest(resource, request_id)
        }
        super::s3_xml::ContentMd5Error::BadDigest => S3ApiError::bad_digest(resource, request_id),
    })?;
    parse_s3_bucket_tagging_xml(content)
        .map_err(|error| map_s3_bucket_tagging_xml_error(error, resource, request_id))
}

fn single_s3_content_md5<'a>(
    headers: &'a HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<Option<&'a [u8]>, S3ApiError> {
    let values = headers.get_all("content-md5").iter().collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => Ok(Some(value.as_bytes())),
        _ => Err(S3ApiError::invalid_argument(
            "Content-MD5 must occur exactly once.",
            resource,
            request_id,
        )),
    }
}

pub(super) fn parse_s3_bucket_tagging_xml(
    input: &[u8],
) -> Result<mediahub_core::S3BucketTagSet, S3BucketTaggingXmlError> {
    if input.is_empty() {
        return Err(S3BucketTaggingXmlError::MalformedXml);
    }
    if input.len() > super::s3_xml::MAX_S3_XML_BODY_BYTES {
        return Err(S3BucketTaggingXmlError::InputTooLarge);
    }

    let mut reader = Reader::from_reader(input);
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut state = S3BucketTaggingXmlState::default();
    let mut declaration_seen = false;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|_| S3BucketTaggingXmlError::MalformedXml)?;
        match event {
            Event::Start(element) => state.enter(&reader, &element)?,
            Event::Empty(element) => {
                let name = element.name().as_ref().to_vec();
                state.enter(&reader, &element)?;
                state.exit(&name)?;
            }
            Event::End(element) => state.exit(element.name().as_ref())?,
            Event::Text(text) => {
                let value = text
                    .xml10_content()
                    .map_err(|_| S3BucketTaggingXmlError::MalformedXml)?;
                state.append_text(&value)?;
            }
            Event::CData(text) => {
                let value = text
                    .xml10_content()
                    .map_err(|_| S3BucketTaggingXmlError::MalformedXml)?;
                state.append_text(&value)?;
            }
            Event::GeneralRef(reference) => {
                let character = if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|_| S3BucketTaggingXmlError::MalformedXml)?
                {
                    character
                } else {
                    match reference
                        .decode()
                        .map_err(|_| S3BucketTaggingXmlError::MalformedXml)?
                        .as_ref()
                    {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(S3BucketTaggingXmlError::MalformedXml),
                    }
                };
                state.append_text(&character.to_string())?;
            }
            Event::Decl(_) if !declaration_seen && !state.root_seen => {
                declaration_seen = true;
            }
            Event::Comment(_) => {}
            Event::DocType(_) | Event::PI(_) | Event::Decl(_) => {
                return Err(S3BucketTaggingXmlError::MalformedXml);
            }
            Event::Eof => break,
        }
        buffer.clear();
    }
    state.finish()
}

pub(super) fn render_s3_bucket_tagging_xml(
    tags: &mediahub_core::S3BucketTagSet,
) -> Result<String, S3BucketTaggingXmlError> {
    tags.validate()
        .map_err(map_s3_bucket_tag_validation_error)
        .map_err(S3BucketTaggingXmlError::InvalidTag)?;
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Tagging xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><TagSet>",
    );
    for tag in tags.iter() {
        output.push_str("<Tag><Key>");
        output.push_str(&escape_s3_xml(tag.key()));
        output.push_str("</Key><Value>");
        output.push_str(&escape_s3_xml(tag.value()));
        output.push_str("</Value></Tag>");
    }
    output.push_str("</TagSet></Tagging>");
    Ok(output)
}

pub(super) fn map_s3_bucket_tagging_xml_error(
    error: S3BucketTaggingXmlError,
    resource: &str,
    request_id: &str,
) -> S3ApiError {
    match error {
        S3BucketTaggingXmlError::InputTooLarge => {
            S3ApiError::entity_too_large(resource, request_id)
        }
        S3BucketTaggingXmlError::MalformedXml => S3ApiError::malformed_xml(resource, request_id),
        S3BucketTaggingXmlError::InvalidTag(error) => {
            S3ApiError::invalid_tag(error.to_string(), resource, request_id)
        }
    }
}

pub(super) fn s3_bucket_tag_set_not_found(resource: &str, request_id: &str) -> S3ApiError {
    S3ApiError::new(
        StatusCode::NOT_FOUND,
        "NoSuchTagSet",
        "The TagSet does not exist.",
        resource,
        request_id,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S3BucketTaggingXmlElement {
    Tagging,
    TagSet,
    Tag,
    Key,
    Value,
}

impl S3BucketTaggingXmlElement {
    const fn name(self) -> &'static [u8] {
        match self {
            Self::Tagging => b"Tagging",
            Self::TagSet => b"TagSet",
            Self::Tag => b"Tag",
            Self::Key => b"Key",
            Self::Value => b"Value",
        }
    }
}

#[derive(Default)]
struct S3BucketTaggingXmlState {
    stack: Vec<S3BucketTaggingXmlElement>,
    root_seen: bool,
    root_closed: bool,
    tag_set_seen: bool,
    current_key: Option<String>,
    current_value: Option<String>,
    tags: Vec<mediahub_core::S3ObjectTag>,
}

impl S3BucketTaggingXmlState {
    fn enter(
        &mut self,
        reader: &Reader<&[u8]>,
        element: &quick_xml::events::BytesStart<'_>,
    ) -> Result<(), S3BucketTaggingXmlError> {
        let name = element.name();
        let name = name.as_ref();
        let kind = match (self.stack.as_slice(), name) {
            ([], b"Tagging") if !self.root_seen && !self.root_closed => {
                validate_s3_bucket_tagging_root_attributes(reader, element)?;
                self.root_seen = true;
                S3BucketTaggingXmlElement::Tagging
            }
            ([S3BucketTaggingXmlElement::Tagging], b"TagSet") if !self.tag_set_seen => {
                validate_s3_bucket_tagging_no_attributes(element)?;
                self.tag_set_seen = true;
                S3BucketTaggingXmlElement::TagSet
            }
            (
                [
                    S3BucketTaggingXmlElement::Tagging,
                    S3BucketTaggingXmlElement::TagSet,
                ],
                b"Tag",
            ) => {
                validate_s3_bucket_tagging_no_attributes(element)?;
                self.current_key = None;
                self.current_value = None;
                S3BucketTaggingXmlElement::Tag
            }
            (
                [
                    S3BucketTaggingXmlElement::Tagging,
                    S3BucketTaggingXmlElement::TagSet,
                    S3BucketTaggingXmlElement::Tag,
                ],
                b"Key",
            ) if self.current_key.is_none() && self.current_value.is_none() => {
                validate_s3_bucket_tagging_no_attributes(element)?;
                self.current_key = Some(String::new());
                S3BucketTaggingXmlElement::Key
            }
            (
                [
                    S3BucketTaggingXmlElement::Tagging,
                    S3BucketTaggingXmlElement::TagSet,
                    S3BucketTaggingXmlElement::Tag,
                ],
                b"Value",
            ) if self.current_key.is_some() && self.current_value.is_none() => {
                validate_s3_bucket_tagging_no_attributes(element)?;
                self.current_value = Some(String::new());
                S3BucketTaggingXmlElement::Value
            }
            _ => return Err(S3BucketTaggingXmlError::MalformedXml),
        };
        self.stack.push(kind);
        Ok(())
    }

    fn exit(&mut self, name: &[u8]) -> Result<(), S3BucketTaggingXmlError> {
        let kind = self
            .stack
            .pop()
            .ok_or(S3BucketTaggingXmlError::MalformedXml)?;
        if kind.name() != name {
            return Err(S3BucketTaggingXmlError::MalformedXml);
        }
        match kind {
            S3BucketTaggingXmlElement::Tag => {
                let key = self
                    .current_key
                    .take()
                    .ok_or(S3BucketTaggingXmlError::MalformedXml)?;
                let value = self
                    .current_value
                    .take()
                    .ok_or(S3BucketTaggingXmlError::MalformedXml)?;
                self.tags.push(
                    mediahub_core::S3ObjectTag::new(key, value)
                        .map_err(map_s3_bucket_tag_validation_error)
                        .map_err(S3BucketTaggingXmlError::InvalidTag)?,
                );
                if self.tags.len() > mediahub_core::MAX_S3_BUCKET_TAGS {
                    return Err(S3BucketTaggingXmlError::InvalidTag(
                        S3BucketTaggingError::TooManyTags,
                    ));
                }
            }
            S3BucketTaggingXmlElement::Tagging => {
                if !self.tag_set_seen {
                    return Err(S3BucketTaggingXmlError::MalformedXml);
                }
                self.root_closed = true;
            }
            S3BucketTaggingXmlElement::TagSet
            | S3BucketTaggingXmlElement::Key
            | S3BucketTaggingXmlElement::Value => {}
        }
        Ok(())
    }

    fn append_text(&mut self, value: &str) -> Result<(), S3BucketTaggingXmlError> {
        match self.stack.last() {
            Some(S3BucketTaggingXmlElement::Key) => self
                .current_key
                .as_mut()
                .ok_or(S3BucketTaggingXmlError::MalformedXml)?
                .push_str(value),
            Some(S3BucketTaggingXmlElement::Value) => self
                .current_value
                .as_mut()
                .ok_or(S3BucketTaggingXmlError::MalformedXml)?
                .push_str(value),
            _ if value.trim().is_empty() => {}
            _ => return Err(S3BucketTaggingXmlError::MalformedXml),
        }
        Ok(())
    }

    fn finish(self) -> Result<mediahub_core::S3BucketTagSet, S3BucketTaggingXmlError> {
        if !self.stack.is_empty()
            || !self.root_seen
            || !self.root_closed
            || !self.tag_set_seen
            || self.current_key.is_some()
            || self.current_value.is_some()
        {
            return Err(S3BucketTaggingXmlError::MalformedXml);
        }
        mediahub_core::S3BucketTagSet::new(self.tags)
            .map_err(map_s3_bucket_tag_validation_error)
            .map_err(S3BucketTaggingXmlError::InvalidTag)
    }
}

fn validate_s3_bucket_tagging_root_attributes(
    reader: &Reader<&[u8]>,
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<(), S3BucketTaggingXmlError> {
    let attributes = element
        .attributes()
        .with_checks(true)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| S3BucketTaggingXmlError::MalformedXml)?;
    if attributes.is_empty() {
        return Ok(());
    }
    if attributes.len() != 1 || attributes[0].key.as_ref() != b"xmlns" {
        return Err(S3BucketTaggingXmlError::MalformedXml);
    }
    let namespace = attributes[0]
        .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
        .map_err(|_| S3BucketTaggingXmlError::MalformedXml)?;
    if namespace != "http://s3.amazonaws.com/doc/2006-03-01/" {
        return Err(S3BucketTaggingXmlError::MalformedXml);
    }
    Ok(())
}

fn validate_s3_bucket_tagging_no_attributes(
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<(), S3BucketTaggingXmlError> {
    if element.attributes().with_checks(true).next().is_some() {
        Err(S3BucketTaggingXmlError::MalformedXml)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod s3_bucket_tagging_protocol_tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use md5::Md5;

    const EMPTY_TAG_SET: &[u8] =
        br#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet/></Tagging>"#;

    fn bucket_tag(key: impl Into<String>, value: impl Into<String>) -> mediahub_core::S3ObjectTag {
        mediahub_core::S3ObjectTag::new(key, value).expect("valid bucket tag")
    }

    fn headers_with_md5(body: &[u8]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let digest = BASE64_STANDARD.encode(Md5::digest(body));
        headers.insert(
            "content-md5",
            HeaderValue::from_str(&digest).expect("MD5 header"),
        );
        headers
    }

    #[test]
    fn bucket_tagging_query_is_exact_and_cannot_mix_subresources() {
        for uri in [
            "/assets?tagging",
            "/assets?tagging=&X-Amz-Algorithm=AWS4-HMAC-SHA256&x-id=GetBucketTagging",
            "/assets?%74agging",
        ] {
            assert!(
                classify_s3_bucket_tagging(&uri.parse().expect("URI"), "request")
                    .expect("tagging query")
            );
        }
        assert!(
            !classify_s3_bucket_tagging(&"/assets".parse().expect("URI"), "request")
                .expect("plain bucket query")
        );
        for uri in [
            "/assets?tagging=value",
            "/assets?tagging&tagging",
            "/assets?tagging&acl",
            "/assets?prefix=images%2F&tagging",
        ] {
            let error = classify_s3_bucket_tagging(&uri.parse().expect("URI"), "request")
                .expect_err("ambiguous Bucket Tagging query");
            assert!(matches!(error.code, "InvalidArgument" | "InvalidRequest"));
        }
    }

    #[test]
    fn bucket_tag_set_accepts_fifty_and_rejects_fifty_one_or_duplicate_keys() {
        let fifty = (0..mediahub_core::MAX_S3_BUCKET_TAGS)
            .map(|index| bucket_tag(format!("key-{index}"), "value"))
            .collect::<Vec<_>>();
        assert_eq!(
            mediahub_core::S3BucketTagSet::new(fifty)
                .expect("50 tags")
                .len(),
            50
        );

        let fifty_one = (0..=mediahub_core::MAX_S3_BUCKET_TAGS)
            .map(|index| bucket_tag(format!("key-{index}"), "value"))
            .collect::<Vec<_>>();
        assert_eq!(
            mediahub_core::S3BucketTagSet::new(fifty_one),
            Err(mediahub_core::S3TaggingError::TooManyTags)
        );
        assert_eq!(
            mediahub_core::S3BucketTagSet::new(vec![
                bucket_tag("same", "one"),
                bucket_tag("same", "two")
            ]),
            Err(mediahub_core::S3TaggingError::DuplicateKey)
        );
    }

    #[test]
    fn bucket_tag_character_limits_are_unicode_counts_and_match_object_tag_rules() {
        assert!(mediahub_core::S3ObjectTag::new("界".repeat(128), "值".repeat(256)).is_ok());
        assert_eq!(
            mediahub_core::S3ObjectTag::new("界".repeat(129), "value"),
            Err(mediahub_core::S3TaggingError::InvalidKey)
        );
        assert_eq!(
            mediahub_core::S3ObjectTag::new("key", "值".repeat(257)),
            Err(mediahub_core::S3TaggingError::InvalidValue)
        );
        for result in [
            mediahub_core::S3ObjectTag::new("", "value"),
            mediahub_core::S3ObjectTag::new("bad&key", "value"),
            mediahub_core::S3ObjectTag::new("key", "line\nbreak"),
        ] {
            assert!(matches!(
                result,
                Err(mediahub_core::S3TaggingError::InvalidKey
                    | mediahub_core::S3TaggingError::InvalidValue)
            ));
        }
    }

    #[test]
    fn empty_tag_set_is_valid_put_replacement_and_round_trips() {
        let tags = parse_s3_bucket_tagging_put(
            &headers_with_md5(EMPTY_TAG_SET),
            EMPTY_TAG_SET,
            "/assets",
            "request",
        )
        .expect("empty replacement TagSet");
        assert!(tags.is_empty());
        let rendered = render_s3_bucket_tagging_xml(&tags).expect("render empty TagSet");
        assert!(rendered.contains("<TagSet></TagSet>"));
        assert!(
            parse_s3_bucket_tagging_xml(rendered.as_bytes())
                .expect("round trip")
                .is_empty()
        );
    }

    #[test]
    fn tagging_xml_is_schema_strict_and_preserves_tag_order() {
        let xml = r#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet><Tag><Key>stage</Key><Value>prod</Value></Tag><Tag><Key>项目</Key><Value>万象仓</Value></Tag></TagSet></Tagging>"#
            .as_bytes();
        let tags = parse_s3_bucket_tagging_xml(xml).expect("valid Bucket Tagging XML");
        assert_eq!(
            tags.iter()
                .map(|tag| (tag.key(), tag.value()))
                .collect::<Vec<_>>(),
            vec![("stage", "prod"), ("项目", "万象仓")]
        );
        let rendered = render_s3_bucket_tagging_xml(&tags).expect("render tags");
        assert!(rendered.find("stage").expect("stage") < rendered.find("项目").expect("项目"));

        assert!(
            parse_s3_bucket_tagging_xml(
                br#"<Tagging><TagSet><Tag><Key>plain</Key><Value>namespace</Value></Tag></TagSet></Tagging>"#
            )
            .is_ok()
        );

        for invalid in [
            br#"<Tagging xmlns="urn:wrong"><TagSet/></Tagging>"#.as_slice(),
            br#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/" extra="x"><TagSet/></Tagging>"#.as_slice(),
            br#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"></Tagging>"#.as_slice(),
            br#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet><Tag><Value>v</Value><Key>k</Key></Tag></TagSet></Tagging>"#.as_slice(),
        ] {
            assert_eq!(
                parse_s3_bucket_tagging_xml(invalid),
                Err(S3BucketTaggingXmlError::MalformedXml)
            );
        }
    }

    #[test]
    fn xml_size_boundary_is_enforced_before_parsing() {
        let mut at_limit = EMPTY_TAG_SET.to_vec();
        at_limit.resize(super::super::s3_xml::MAX_S3_XML_BODY_BYTES, b' ');
        assert!(parse_s3_bucket_tagging_xml(&at_limit).is_ok());
        at_limit.push(b' ');
        assert_eq!(
            parse_s3_bucket_tagging_xml(&at_limit),
            Err(S3BucketTaggingXmlError::InputTooLarge)
        );
    }

    #[test]
    fn content_md5_and_protocol_errors_map_to_s3_codes() {
        let no_headers = HeaderMap::new();
        assert_eq!(
            parse_s3_bucket_tagging_put(&no_headers, EMPTY_TAG_SET, "/assets", "request")
                .expect_err("missing Content-MD5")
                .code,
            "InvalidDigest"
        );

        let mut duplicate_digest = headers_with_md5(EMPTY_TAG_SET);
        duplicate_digest.append(
            "content-md5",
            HeaderValue::from_static("AAAAAAAAAAAAAAAAAAAAAA=="),
        );
        assert_eq!(
            parse_s3_bucket_tagging_put(
                &duplicate_digest,
                EMPTY_TAG_SET,
                "/assets",
                "request"
            )
            .expect_err("duplicate Content-MD5")
            .code,
            "InvalidArgument"
        );

        let mut wrong_digest = HeaderMap::new();
        wrong_digest.insert(
            "content-md5",
            HeaderValue::from_static("AAAAAAAAAAAAAAAAAAAAAA=="),
        );
        assert_eq!(
            parse_s3_bucket_tagging_put(&wrong_digest, EMPTY_TAG_SET, "/assets", "request")
                .expect_err("wrong Content-MD5")
                .code,
            "BadDigest"
        );

        let duplicate = br#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet><Tag><Key>same</Key><Value>one</Value></Tag><Tag><Key>same</Key><Value>two</Value></Tag></TagSet></Tagging>"#;
        assert_eq!(
            parse_s3_bucket_tagging_put(
                &headers_with_md5(duplicate),
                duplicate,
                "/assets",
                "request"
            )
            .expect_err("duplicate key")
            .code,
            "InvalidTag"
        );
        assert_eq!(
            s3_bucket_tag_set_not_found("/assets", "request").code,
            "NoSuchTagSet"
        );

        for (source, expected_status, expected_code) in [
            (
                S3BucketTaggingXmlError::InputTooLarge,
                StatusCode::PAYLOAD_TOO_LARGE,
                "EntityTooLarge",
            ),
            (
                S3BucketTaggingXmlError::MalformedXml,
                StatusCode::BAD_REQUEST,
                "MalformedXML",
            ),
            (
                S3BucketTaggingXmlError::InvalidTag(S3BucketTaggingError::DuplicateKey),
                StatusCode::BAD_REQUEST,
                "InvalidTag",
            ),
        ] {
            let error = map_s3_bucket_tagging_xml_error(source, "/assets", "request");
            assert_eq!(error.status, expected_status);
            assert_eq!(error.code, expected_code);
        }
    }
}
