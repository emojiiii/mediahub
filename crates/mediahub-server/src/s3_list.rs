use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng, Payload, rand_core::RngCore},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use thiserror::Error;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

const TOKEN_VERSION: u8 = 1;
const TOKEN_NONCE_BYTES: usize = 12;
const TOKEN_TAG_BYTES: usize = 16;
const MAX_TOKEN_BYTES: usize = 8 * 1024;
const MAX_INTERNAL_CURSOR_BYTES: usize = 4 * 1024;
const MAX_KEY_BYTES: usize = 1024;
const MAX_KEYS: usize = 1000;
const TOKEN_AAD_LABEL: &[u8] = b"mediahub:s3:list-objects-v2:continuation:v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListEncodingType {
    Url,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListObjectsV2Query {
    pub(crate) prefix: String,
    pub(crate) delimiter: Option<String>,
    pub(crate) max_keys: usize,
    pub(crate) continuation_token: Option<String>,
    pub(crate) start_after: Option<String>,
    pub(crate) encoding_type: Option<ListEncodingType>,
}

impl ListObjectsV2Query {
    pub(crate) fn parse(raw_query: Option<&str>) -> Result<Self, S3ListError> {
        let mut list_type = None;
        let mut prefix = None;
        let mut delimiter = None;
        let mut max_keys = None;
        let mut continuation_token = None;
        let mut start_after = None;
        let mut encoding_type = None;

        for (name, value) in url::form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes()) {
            match name.as_ref() {
                "list-type" => set_once(&mut list_type, value.into_owned(), "list-type")?,
                "prefix" => set_once(&mut prefix, value.into_owned(), "prefix")?,
                "delimiter" => set_once(&mut delimiter, value.into_owned(), "delimiter")?,
                "max-keys" => set_once(&mut max_keys, value.into_owned(), "max-keys")?,
                "continuation-token" => set_once(
                    &mut continuation_token,
                    value.into_owned(),
                    "continuation-token",
                )?,
                "start-after" => {
                    set_once(&mut start_after, value.into_owned(), "start-after")?;
                }
                "encoding-type" => {
                    set_once(&mut encoding_type, value.into_owned(), "encoding-type")?;
                }
                _ => {}
            }
        }

        match list_type.as_deref() {
            None => return Err(S3ListError::MissingListType),
            Some("2") => {}
            Some(_) => return Err(S3ListError::UnsupportedListType),
        }

        let prefix = prefix.unwrap_or_default();
        validate_key_like(&prefix).map_err(|()| S3ListError::InvalidPrefix)?;
        let delimiter = match delimiter {
            None => None,
            Some(value) if value.is_empty() => None,
            Some(value) if value == "/" => Some(value),
            Some(_) => return Err(S3ListError::InvalidDelimiter),
        };
        let max_keys = max_keys
            .as_deref()
            .map(parse_max_keys)
            .transpose()?
            .unwrap_or(MAX_KEYS);
        let continuation_token = continuation_token
            .map(|value| {
                if value.is_empty() || value.len() > MAX_TOKEN_BYTES {
                    Err(S3ListError::InvalidContinuationToken)
                } else {
                    Ok(value)
                }
            })
            .transpose()?;
        let start_after = start_after
            .map(|value| {
                validate_key_like(&value).map_err(|()| S3ListError::InvalidStartAfter)?;
                Ok(value)
            })
            .transpose()?;
        if continuation_token.is_some() && start_after.is_some() {
            return Err(S3ListError::ConflictingCursors);
        }
        let encoding_type = match encoding_type.as_deref() {
            None => None,
            Some("url") => Some(ListEncodingType::Url),
            Some(_) => return Err(S3ListError::InvalidEncodingType),
        };

        Ok(Self {
            prefix,
            delimiter,
            max_keys,
            continuation_token,
            start_after,
            encoding_type,
        })
    }

    pub(crate) fn decode_continuation_cursor(
        &self,
        codec: &ContinuationTokenCodec,
        bucket: &str,
    ) -> Result<Option<String>, S3ListError> {
        self.continuation_token
            .as_deref()
            .map(|token| codec.decode(token, bucket, self))
            .transpose()
    }
}

fn set_once(
    slot: &mut Option<String>,
    value: String,
    name: &'static str,
) -> Result<(), S3ListError> {
    if slot.replace(value).is_some() {
        return Err(S3ListError::DuplicateParameter(name));
    }
    Ok(())
}

fn parse_max_keys(value: &str) -> Result<usize, S3ListError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(S3ListError::InvalidMaxKeys);
    }
    let value = value
        .parse::<usize>()
        .map_err(|_| S3ListError::InvalidMaxKeys)?;
    if value > MAX_KEYS {
        return Err(S3ListError::InvalidMaxKeys);
    }
    Ok(value)
}

fn validate_key_like(value: &str) -> Result<(), ()> {
    if value.len() > MAX_KEY_BYTES || value.as_bytes().contains(&0) {
        return Err(());
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct ContinuationTokenCodec {
    cipher: Aes256Gcm,
}

impl ContinuationTokenCodec {
    #[must_use]
    pub(crate) fn new(key: [u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(&key).expect("AES-256 key has a fixed valid length"),
        }
    }

    pub(crate) fn encode(
        &self,
        bucket: &str,
        query: &ListObjectsV2Query,
        internal_cursor: &str,
    ) -> Result<String, S3ListError> {
        if internal_cursor.is_empty() || internal_cursor.len() > MAX_INTERNAL_CURSOR_BYTES {
            return Err(S3ListError::InvalidInternalCursor);
        }
        let mut nonce_bytes = [0_u8; TOKEN_NONCE_BYTES];
        OsRng.fill_bytes(&mut nonce_bytes);
        let aad = token_aad(bucket, query)?;
        let ciphertext = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: internal_cursor.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| S3ListError::TokenEncodingFailed)?;
        let mut bytes = Vec::with_capacity(1 + TOKEN_NONCE_BYTES + ciphertext.len());
        bytes.push(TOKEN_VERSION);
        bytes.extend_from_slice(&nonce_bytes);
        bytes.extend_from_slice(&ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub(crate) fn decode(
        &self,
        token: &str,
        bucket: &str,
        query: &ListObjectsV2Query,
    ) -> Result<String, S3ListError> {
        if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
            return Err(S3ListError::InvalidContinuationToken);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| S3ListError::InvalidContinuationToken)?;
        if bytes.len() < 1 + TOKEN_NONCE_BYTES + TOKEN_TAG_BYTES + 1
            || bytes.first() != Some(&TOKEN_VERSION)
        {
            return Err(S3ListError::InvalidContinuationToken);
        }
        let nonce = &bytes[1..1 + TOKEN_NONCE_BYTES];
        let ciphertext = &bytes[1 + TOKEN_NONCE_BYTES..];
        let aad = token_aad(bucket, query)?;
        let cursor = self
            .cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| S3ListError::InvalidContinuationToken)?;
        if cursor.is_empty() || cursor.len() > MAX_INTERNAL_CURSOR_BYTES {
            return Err(S3ListError::InvalidContinuationToken);
        }
        String::from_utf8(cursor).map_err(|_| S3ListError::InvalidContinuationToken)
    }
}

fn token_aad(bucket: &str, query: &ListObjectsV2Query) -> Result<Vec<u8>, S3ListError> {
    if bucket.is_empty() {
        return Err(S3ListError::InvalidBucketContext);
    }
    let mut aad = Vec::with_capacity(
        TOKEN_AAD_LABEL.len()
            + bucket.len()
            + query.prefix.len()
            + query.delimiter.as_ref().map_or(0, String::len)
            + 16,
    );
    aad.extend_from_slice(TOKEN_AAD_LABEL);
    push_context_component(&mut aad, bucket)?;
    push_context_component(&mut aad, &query.prefix)?;
    push_context_component(&mut aad, query.delimiter.as_deref().unwrap_or_default())?;
    Ok(aad)
}

fn push_context_component(output: &mut Vec<u8>, value: &str) -> Result<(), S3ListError> {
    let length = u32::try_from(value.len()).map_err(|_| S3ListError::InvalidBucketContext)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListObject {
    pub(crate) key: String,
    pub(crate) last_modified: OffsetDateTime,
    pub(crate) etag: String,
    pub(crate) size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListObjectsV2Result {
    pub(crate) bucket: String,
    pub(crate) query: ListObjectsV2Query,
    pub(crate) items: Vec<ListObject>,
    pub(crate) common_prefixes: Vec<String>,
    pub(crate) next_cursor: Option<String>,
}

impl ListObjectsV2Result {
    pub(crate) fn to_xml(&self, codec: &ContinuationTokenCodec) -> Result<String, S3ListError> {
        let max_keys_is_zero = self.query.max_keys == 0;
        let key_count = if max_keys_is_zero {
            0
        } else {
            self.items.len() + self.common_prefixes.len()
        };
        if key_count > self.query.max_keys {
            return Err(S3ListError::PageExceedsMaxKeys);
        }
        let next_token = if max_keys_is_zero {
            None
        } else {
            self.next_cursor
                .as_deref()
                .map(|cursor| codec.encode(&self.bucket, &self.query, cursor))
                .transpose()?
        };

        let mut xml = String::with_capacity(512 + key_count.saturating_mul(256));
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
        xml.push_str("<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">");
        push_element(&mut xml, "Name", &self.bucket)?;
        push_encoded_element(
            &mut xml,
            "Prefix",
            &self.query.prefix,
            self.query.encoding_type,
        )?;
        push_element(&mut xml, "KeyCount", &key_count.to_string())?;
        push_element(&mut xml, "MaxKeys", &self.query.max_keys.to_string())?;
        if let Some(delimiter) = self.query.delimiter.as_deref() {
            push_encoded_element(&mut xml, "Delimiter", delimiter, self.query.encoding_type)?;
        }
        if self.query.encoding_type == Some(ListEncodingType::Url) {
            push_element(&mut xml, "EncodingType", "url")?;
        }
        push_element(
            &mut xml,
            "IsTruncated",
            if next_token.is_some() {
                "true"
            } else {
                "false"
            },
        )?;

        if !max_keys_is_zero {
            for item in &self.items {
                xml.push_str("<Contents>");
                push_encoded_element(&mut xml, "Key", &item.key, self.query.encoding_type)?;
                let last_modified = item
                    .last_modified
                    .to_offset(UtcOffset::UTC)
                    .format(&Rfc3339)
                    .map_err(|_| S3ListError::InvalidLastModified)?;
                push_element(&mut xml, "LastModified", &last_modified)?;
                let etag = if item.etag.starts_with('"') && item.etag.ends_with('"') {
                    item.etag.clone()
                } else {
                    format!("\"{}\"", item.etag)
                };
                push_element(&mut xml, "ETag", &etag)?;
                push_element(&mut xml, "Size", &item.size.to_string())?;
                push_element(&mut xml, "StorageClass", "STANDARD")?;
                xml.push_str("</Contents>");
            }
            for prefix in &self.common_prefixes {
                xml.push_str("<CommonPrefixes>");
                push_encoded_element(&mut xml, "Prefix", prefix, self.query.encoding_type)?;
                xml.push_str("</CommonPrefixes>");
            }
        }

        if let Some(token) = self.query.continuation_token.as_deref() {
            push_element(&mut xml, "ContinuationToken", token)?;
        }
        if let Some(token) = next_token.as_deref() {
            push_element(&mut xml, "NextContinuationToken", token)?;
        }
        if let Some(start_after) = self.query.start_after.as_deref() {
            push_encoded_element(
                &mut xml,
                "StartAfter",
                start_after,
                self.query.encoding_type,
            )?;
        }
        xml.push_str("</ListBucketResult>");
        Ok(xml)
    }
}

fn push_encoded_element(
    output: &mut String,
    name: &str,
    value: &str,
    encoding_type: Option<ListEncodingType>,
) -> Result<(), S3ListError> {
    if encoding_type == Some(ListEncodingType::Url) {
        push_element(output, name, &s3_url_encode(value))
    } else {
        push_element(output, name, value)
    }
}

fn push_element(output: &mut String, name: &str, value: &str) -> Result<(), S3ListError> {
    output.push('<');
    output.push_str(name);
    output.push('>');
    escape_xml_text(output, value)?;
    output.push_str("</");
    output.push_str(name);
    output.push('>');
    Ok(())
}

fn escape_xml_text(output: &mut String, value: &str) -> Result<(), S3ListError> {
    for character in value.chars() {
        if !is_xml_1_0_character(character) {
            return Err(S3ListError::InvalidXmlCharacter);
        }
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            character => output.push(character),
        }
    }
    Ok(())
}

fn is_xml_1_0_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&character)
        || ('\u{E000}'..='\u{FFFD}').contains(&character)
        || ('\u{10000}'..='\u{10FFFF}').contains(&character)
}

fn s3_url_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum S3ListError {
    #[error("list-type is required")]
    MissingListType,
    #[error("only list-type=2 is supported")]
    UnsupportedListType,
    #[error("query parameter {0} must not occur more than once")]
    DuplicateParameter(&'static str),
    #[error("delimiter must be empty or /")]
    InvalidDelimiter,
    #[error("max-keys must be an integer between 0 and 1000")]
    InvalidMaxKeys,
    #[error("encoding-type must be url")]
    InvalidEncodingType,
    #[error("prefix is invalid")]
    InvalidPrefix,
    #[error("start-after is invalid")]
    InvalidStartAfter,
    #[error("continuation-token is invalid")]
    InvalidContinuationToken,
    #[error("continuation-token and start-after cannot be used together")]
    ConflictingCursors,
    #[error("the continuation token bucket context is invalid")]
    InvalidBucketContext,
    #[error("the internal continuation cursor is invalid")]
    InvalidInternalCursor,
    #[error("continuation token encoding failed")]
    TokenEncodingFailed,
    #[error("the result page contains more entries than max-keys")]
    PageExceedsMaxKeys,
    #[error("last-modified cannot be represented as an S3 timestamp")]
    InvalidLastModified,
    #[error("response contains a character that XML 1.0 cannot represent")]
    InvalidXmlCharacter,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(raw: &str) -> ListObjectsV2Query {
        ListObjectsV2Query::parse(Some(raw)).expect("valid query")
    }

    fn item(key: &str) -> ListObject {
        ListObject {
            key: key.to_owned(),
            last_modified: OffsetDateTime::from_unix_timestamp(1_700_000_000)
                .expect("valid timestamp"),
            etag: "abc123".to_owned(),
            size: 42,
        }
    }

    #[test]
    fn parses_supported_query_and_defaults() {
        let parsed = query("list-type=2");
        assert_eq!(parsed.prefix, "");
        assert_eq!(parsed.delimiter, None);
        assert_eq!(parsed.max_keys, 1000);
        assert_eq!(parsed.encoding_type, None);

        let parsed = query(
            "list-type=2&prefix=images%2F%E5%A4%B4%E5%83%8F%2F&delimiter=%2F&max-keys=0&start-after=images%2Fa.png&encoding-type=url",
        );
        assert_eq!(parsed.prefix, "images/头像/");
        assert_eq!(parsed.delimiter.as_deref(), Some("/"));
        assert_eq!(parsed.max_keys, 0);
        assert_eq!(parsed.start_after.as_deref(), Some("images/a.png"));
        assert_eq!(parsed.encoding_type, Some(ListEncodingType::Url));

        assert_eq!(query("list-type=2&delimiter=").delimiter, None);
    }

    #[test]
    fn rejects_invalid_or_ambiguous_query_parameters() {
        assert_eq!(
            ListObjectsV2Query::parse(None),
            Err(S3ListError::MissingListType)
        );
        assert_eq!(
            ListObjectsV2Query::parse(Some("list-type=1")),
            Err(S3ListError::UnsupportedListType)
        );
        assert_eq!(
            ListObjectsV2Query::parse(Some("list-type=2&list-type=2")),
            Err(S3ListError::DuplicateParameter("list-type"))
        );
        assert_eq!(
            ListObjectsV2Query::parse(Some("list-type=2&delimiter=|")),
            Err(S3ListError::InvalidDelimiter)
        );
        for invalid in ["", "-1", "+1", "1001", "184467440737095516160"] {
            assert_eq!(
                ListObjectsV2Query::parse(Some(&format!("list-type=2&max-keys={invalid}"))),
                Err(S3ListError::InvalidMaxKeys)
            );
        }
        assert_eq!(
            ListObjectsV2Query::parse(Some("list-type=2&encoding-type=xml")),
            Err(S3ListError::InvalidEncodingType)
        );
        assert_eq!(
            ListObjectsV2Query::parse(Some("list-type=2&continuation-token=token&start-after=key")),
            Err(S3ListError::ConflictingCursors)
        );
    }

    #[test]
    fn continuation_token_round_trips_without_disclosing_cursor() {
        let codec = ContinuationTokenCodec::new([7; 32]);
        let parsed = query("list-type=2&prefix=images%2F&delimiter=%2F");
        let cursor = "DATABASE-CURSOR-DO-NOT-LEAK";
        let token = codec
            .encode("media", &parsed, cursor)
            .expect("encode token");
        assert!(!token.contains(cursor));
        let raw = URL_SAFE_NO_PAD.decode(&token).expect("base64 token");
        assert!(
            !raw.windows(cursor.len())
                .any(|window| window == cursor.as_bytes())
        );
        assert_eq!(
            codec
                .decode(&token, "media", &parsed)
                .expect("decode token"),
            cursor
        );
    }

    #[test]
    fn continuation_token_rejects_tampering_and_context_changes() {
        let codec = ContinuationTokenCodec::new([9; 32]);
        let parsed = query("list-type=2&prefix=images%2F&delimiter=%2F");
        let token = codec
            .encode("media", &parsed, "cursor")
            .expect("encode token");

        let mut bytes = URL_SAFE_NO_PAD.decode(&token).expect("base64 token");
        let last = bytes.last_mut().expect("token bytes");
        *last ^= 1;
        let tampered = URL_SAFE_NO_PAD.encode(bytes);
        assert_eq!(
            codec.decode(&tampered, "media", &parsed),
            Err(S3ListError::InvalidContinuationToken)
        );
        assert_eq!(
            codec.decode(&token, "other", &parsed),
            Err(S3ListError::InvalidContinuationToken)
        );
        let other_prefix = query("list-type=2&prefix=other%2F&delimiter=%2F");
        assert_eq!(
            codec.decode(&token, "media", &other_prefix),
            Err(S3ListError::InvalidContinuationToken)
        );
        let no_delimiter = query("list-type=2&prefix=images%2F");
        assert_eq!(
            codec.decode(&token, "media", &no_delimiter),
            Err(S3ListError::InvalidContinuationToken)
        );
    }

    #[test]
    fn renders_s3_xml_with_escaping_and_an_opaque_next_token() {
        let codec = ContinuationTokenCodec::new([3; 32]);
        let result = ListObjectsV2Result {
            bucket: "media&amp".to_owned(),
            query: query(
                "list-type=2&prefix=p%3C%26%3E%2F&delimiter=%2F&max-keys=3&start-after=before%26",
            ),
            items: vec![item("p<&>/image.png")],
            common_prefixes: vec!["p<&>/folder/".to_owned()],
            next_cursor: Some("internal-cursor".to_owned()),
        };
        let xml = result.to_xml(&codec).expect("render XML");
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<Name>media&amp;amp</Name>"));
        assert!(xml.contains("<Prefix>p&lt;&amp;&gt;/</Prefix>"));
        assert!(xml.contains("<KeyCount>2</KeyCount>"));
        assert!(xml.contains("<IsTruncated>true</IsTruncated>"));
        assert!(xml.contains("<Key>p&lt;&amp;&gt;/image.png</Key>"));
        assert!(xml.contains("<LastModified>2023-11-14T22:13:20Z</LastModified>"));
        assert!(xml.contains("<ETag>&quot;abc123&quot;</ETag>"));
        assert!(xml.contains("<Size>42</Size><StorageClass>STANDARD</StorageClass>"));
        assert!(xml.contains("<CommonPrefixes><Prefix>p&lt;&amp;&gt;/folder/</Prefix>"));
        assert!(xml.contains("<StartAfter>before&amp;</StartAfter>"));
        assert!(xml.contains("<NextContinuationToken>"));
    }

    #[test]
    fn url_encoding_covers_utf8_spaces_and_slashes() {
        let codec = ContinuationTokenCodec::new([4; 32]);
        let result = ListObjectsV2Result {
            bucket: "media".to_owned(),
            query: query(
                "list-type=2&prefix=%E5%9B%BE%E7%89%87%2F&delimiter=%2F&max-keys=2&encoding-type=url&start-after=a%20b",
            ),
            items: vec![item("图片/a b.png")],
            common_prefixes: vec![],
            next_cursor: None,
        };
        let xml = result.to_xml(&codec).expect("render XML");
        assert!(xml.contains("<Prefix>%E5%9B%BE%E7%89%87%2F</Prefix>"));
        assert!(xml.contains("<Delimiter>%2F</Delimiter>"));
        assert!(xml.contains("<EncodingType>url</EncodingType>"));
        assert!(xml.contains("<Key>%E5%9B%BE%E7%89%87%2Fa%20b.png</Key>"));
        assert!(xml.contains("<StartAfter>a%20b</StartAfter>"));
    }

    #[test]
    fn max_keys_zero_always_renders_an_empty_non_truncated_page() {
        let codec = ContinuationTokenCodec::new([5; 32]);
        let result = ListObjectsV2Result {
            bucket: "media".to_owned(),
            query: query("list-type=2&max-keys=0"),
            items: vec![item("must-not-appear")],
            common_prefixes: vec!["must-not-appear/".to_owned()],
            next_cursor: Some("must-not-appear".to_owned()),
        };
        let xml = result.to_xml(&codec).expect("render XML");
        assert!(xml.contains("<KeyCount>0</KeyCount>"));
        assert!(xml.contains("<MaxKeys>0</MaxKeys>"));
        assert!(xml.contains("<IsTruncated>false</IsTruncated>"));
        assert!(!xml.contains("<Contents>"));
        assert!(!xml.contains("<CommonPrefixes>"));
        assert!(!xml.contains("<NextContinuationToken>"));
        assert!(!xml.contains("must-not-appear"));
    }

    #[test]
    fn xml_control_characters_require_url_encoding() {
        let codec = ContinuationTokenCodec::new([6; 32]);
        let result = ListObjectsV2Result {
            bucket: "media".to_owned(),
            query: query("list-type=2&max-keys=1"),
            items: vec![item("bad\u{1}key")],
            common_prefixes: vec![],
            next_cursor: None,
        };
        assert_eq!(result.to_xml(&codec), Err(S3ListError::InvalidXmlCharacter));

        let mut encoded_result = result;
        encoded_result.query.encoding_type = Some(ListEncodingType::Url);
        assert!(
            encoded_result
                .to_xml(&codec)
                .expect("URL encoding makes XML valid")
                .contains("<Key>bad%01key</Key>")
        );
    }
}
