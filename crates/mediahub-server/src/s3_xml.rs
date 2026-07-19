use std::fmt::Write as _;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use md5::{Digest as _, Md5};
use quick_xml::{events::Event, reader::Reader};
use thiserror::Error;

const S3_XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";
const ALL_USERS_GROUP: &str = "http://acs.amazonaws.com/groups/global/AllUsers";
pub(crate) const MAX_S3_XML_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_XML_DEPTH: usize = 16;
const MAX_XML_NODES: usize = 40_016;
const MAX_DELETE_OBJECTS: usize = 1_000;
const MAX_MULTIPART_PARTS: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeleteObjectIdentifier {
    pub(crate) key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeleteObjectsRequest {
    pub(crate) objects: Vec<DeleteObjectIdentifier>,
    pub(crate) quiet: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompletedPart {
    pub(crate) part_number: u16,
    pub(crate) etag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompleteMultipartUploadRequest {
    pub(crate) parts: Vec<CompletedPart>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeletedObject {
    pub(crate) key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeleteObjectError {
    pub(crate) key: String,
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeleteResult {
    pub(crate) deleted: Vec<DeletedObject>,
    pub(crate) errors: Vec<DeleteObjectError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListedPart {
    pub(crate) part_number: u16,
    pub(crate) last_modified: String,
    pub(crate) etag: String,
    pub(crate) size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListPartsResult {
    pub(crate) bucket: String,
    pub(crate) key: String,
    pub(crate) upload_id: String,
    pub(crate) part_number_marker: u16,
    pub(crate) next_part_number_marker: u16,
    pub(crate) max_parts: u16,
    pub(crate) is_truncated: bool,
    pub(crate) parts: Vec<ListedPart>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectAcl {
    Private,
    PublicRead,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum ContentMd5Error {
    #[error("the Content-MD5 header is missing or invalid")]
    InvalidDigest,
    #[error("the Content-MD5 header does not match the request body")]
    BadDigest,
}

impl ContentMd5Error {
    pub(crate) const fn s3_code(self) -> &'static str {
        match self {
            Self::InvalidDigest => "InvalidDigest",
            Self::BadDigest => "BadDigest",
        }
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum S3XmlError {
    #[error("the XML request body exceeds the supported limit")]
    InputTooLarge,
    #[error("the XML request body is malformed")]
    MalformedXml,
    #[error("a DeleteObjects request cannot contain more than 1000 objects")]
    TooManyObjects,
    #[error("each object must contain a non-empty Key")]
    MissingKey,
    #[error("VersionId is not supported")]
    VersionIdNotSupported,
    #[error("each multipart part must contain a PartNumber")]
    MissingPartNumber,
    #[error("multipart PartNumber must be between 1 and 10000")]
    InvalidPartNumber,
    #[error("each multipart part must contain a non-empty ETag")]
    MissingEtag,
    #[error("multipart parts must be strictly ordered by PartNumber")]
    InvalidPartOrder,
    #[error("a value contains a character that XML 1.0 cannot represent")]
    InvalidXmlCharacter,
}

impl S3XmlError {
    pub(crate) const fn s3_code(self) -> &'static str {
        match self {
            Self::VersionIdNotSupported => "InvalidRequest",
            Self::InputTooLarge => "EntityTooLarge",
            Self::MalformedXml
            | Self::TooManyObjects
            | Self::MissingKey
            | Self::MissingPartNumber
            | Self::InvalidPartNumber
            | Self::MissingEtag
            | Self::InvalidPartOrder
            | Self::InvalidXmlCharacter => "MalformedXML",
        }
    }
}

pub(crate) fn validate_content_md5(
    content_md5: Option<&[u8]>,
    body: &[u8],
) -> Result<(), ContentMd5Error> {
    let encoded = content_md5.ok_or(ContentMd5Error::InvalidDigest)?;
    let decoded = BASE64_STANDARD
        .decode(encoded)
        .map_err(|_| ContentMd5Error::InvalidDigest)?;
    let expected: [u8; 16] = decoded
        .try_into()
        .map_err(|_| ContentMd5Error::InvalidDigest)?;
    let actual: [u8; 16] = Md5::digest(body).into();
    if actual == expected {
        Ok(())
    } else {
        Err(ContentMd5Error::BadDigest)
    }
}

pub(crate) fn parse_delete_objects_xml(input: &[u8]) -> Result<DeleteObjectsRequest, S3XmlError> {
    let root = parse_xml_document(input)?;
    if root.name != "Delete" || !root.text.trim().is_empty() {
        return Err(S3XmlError::MalformedXml);
    }

    let mut objects = Vec::new();
    let mut quiet = None;
    for child in &root.children {
        match child.name.as_str() {
            "Object" => {
                if !child.text.trim().is_empty() {
                    return Err(S3XmlError::MalformedXml);
                }
                let mut key = None;
                for field in &child.children {
                    if field.name == "VersionId" {
                        return Err(S3XmlError::VersionIdNotSupported);
                    }
                    if !field.children.is_empty() {
                        return Err(S3XmlError::MalformedXml);
                    }
                    match field.name.as_str() {
                        "Key" if key.is_none() => key = Some(field.text.clone()),
                        _ => return Err(S3XmlError::MalformedXml),
                    }
                }
                let key = key
                    .filter(|value| !value.is_empty())
                    .ok_or(S3XmlError::MissingKey)?;
                if objects.len() == MAX_DELETE_OBJECTS {
                    return Err(S3XmlError::TooManyObjects);
                }
                objects.push(DeleteObjectIdentifier { key });
            }
            "Quiet" if quiet.is_none() && child.children.is_empty() => {
                quiet = match child.text.trim() {
                    value if value.eq_ignore_ascii_case("true") => Some(true),
                    value if value.eq_ignore_ascii_case("false") => Some(false),
                    _ => return Err(S3XmlError::MalformedXml),
                };
            }
            _ => return Err(S3XmlError::MalformedXml),
        }
    }
    if objects.is_empty() {
        return Err(S3XmlError::MissingKey);
    }
    Ok(DeleteObjectsRequest {
        objects,
        quiet: quiet.unwrap_or(false),
    })
}

pub(crate) fn parse_complete_multipart_upload_xml(
    input: &[u8],
) -> Result<CompleteMultipartUploadRequest, S3XmlError> {
    let root = parse_xml_document(input)?;
    if root.name != "CompleteMultipartUpload" || !root.text.trim().is_empty() {
        return Err(S3XmlError::MalformedXml);
    }

    let mut parts = Vec::new();
    let mut previous_part_number = 0_u16;
    for child in &root.children {
        if child.name != "Part" || !child.text.trim().is_empty() {
            return Err(S3XmlError::MalformedXml);
        }
        let mut part_number = None;
        let mut etag = None;
        for field in &child.children {
            if !field.children.is_empty() {
                return Err(S3XmlError::MalformedXml);
            }
            match field.name.as_str() {
                "PartNumber" if part_number.is_none() => {
                    let parsed = field
                        .text
                        .trim()
                        .parse::<u16>()
                        .map_err(|_| S3XmlError::InvalidPartNumber)?;
                    if parsed == 0 || usize::from(parsed) > MAX_MULTIPART_PARTS {
                        return Err(S3XmlError::InvalidPartNumber);
                    }
                    part_number = Some(parsed);
                }
                "ETag" if etag.is_none() => {
                    let value = field.text.trim();
                    if value.is_empty() {
                        return Err(S3XmlError::MissingEtag);
                    }
                    etag = Some(value.to_owned());
                }
                _ => return Err(S3XmlError::MalformedXml),
            }
        }
        let part_number = part_number.ok_or(S3XmlError::MissingPartNumber)?;
        if part_number <= previous_part_number {
            return Err(S3XmlError::InvalidPartOrder);
        }
        previous_part_number = part_number;
        parts.push(CompletedPart {
            part_number,
            etag: etag.ok_or(S3XmlError::MissingEtag)?,
        });
    }
    if parts.is_empty() {
        return Err(S3XmlError::MissingPartNumber);
    }
    Ok(CompleteMultipartUploadRequest { parts })
}

pub(crate) fn delete_result_xml(result: &DeleteResult) -> Result<String, S3XmlError> {
    let mut output = xml_document_start("DeleteResult");
    for deleted in &result.deleted {
        output.push_str("<Deleted>");
        push_element(&mut output, "Key", &deleted.key)?;
        output.push_str("</Deleted>");
    }
    for error in &result.errors {
        output.push_str("<Error>");
        push_element(&mut output, "Key", &error.key)?;
        push_element(&mut output, "Code", &error.code)?;
        push_element(&mut output, "Message", &error.message)?;
        output.push_str("</Error>");
    }
    output.push_str("</DeleteResult>");
    Ok(output)
}

pub(crate) fn initiate_multipart_upload_result_xml(
    bucket: &str,
    key: &str,
    upload_id: &str,
) -> Result<String, S3XmlError> {
    let mut output = xml_document_start("InitiateMultipartUploadResult");
    push_element(&mut output, "Bucket", bucket)?;
    push_element(&mut output, "Key", key)?;
    push_element(&mut output, "UploadId", upload_id)?;
    output.push_str("</InitiateMultipartUploadResult>");
    Ok(output)
}

pub(crate) fn list_parts_result_xml(result: &ListPartsResult) -> Result<String, S3XmlError> {
    let mut output = xml_document_start("ListPartsResult");
    push_element(&mut output, "Bucket", &result.bucket)?;
    push_element(&mut output, "Key", &result.key)?;
    push_element(&mut output, "UploadId", &result.upload_id)?;
    push_number_element(&mut output, "PartNumberMarker", result.part_number_marker);
    push_number_element(
        &mut output,
        "NextPartNumberMarker",
        result.next_part_number_marker,
    );
    push_number_element(&mut output, "MaxParts", result.max_parts);
    push_element(
        &mut output,
        "IsTruncated",
        if result.is_truncated { "true" } else { "false" },
    )?;
    for part in &result.parts {
        output.push_str("<Part>");
        push_number_element(&mut output, "PartNumber", part.part_number);
        push_element(&mut output, "LastModified", &part.last_modified)?;
        push_element(&mut output, "ETag", &part.etag)?;
        push_number_element(&mut output, "Size", part.size);
        output.push_str("</Part>");
    }
    output.push_str("</ListPartsResult>");
    Ok(output)
}

pub(crate) fn complete_multipart_upload_result_xml(
    location: &str,
    bucket: &str,
    key: &str,
    etag: &str,
) -> Result<String, S3XmlError> {
    let mut output = xml_document_start("CompleteMultipartUploadResult");
    push_element(&mut output, "Location", location)?;
    push_element(&mut output, "Bucket", bucket)?;
    push_element(&mut output, "Key", key)?;
    push_element(&mut output, "ETag", etag)?;
    output.push_str("</CompleteMultipartUploadResult>");
    Ok(output)
}

pub(crate) fn get_object_acl_xml(
    owner_id: &str,
    owner_display_name: &str,
    acl: ObjectAcl,
) -> Result<String, S3XmlError> {
    let mut output = xml_document_start("AccessControlPolicy");
    output.push_str("<Owner>");
    push_element(&mut output, "ID", owner_id)?;
    push_element(&mut output, "DisplayName", owner_display_name)?;
    output.push_str("</Owner><AccessControlList><Grant>");
    output.push_str(
        "<Grantee xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"CanonicalUser\">",
    );
    push_element(&mut output, "ID", owner_id)?;
    push_element(&mut output, "DisplayName", owner_display_name)?;
    output.push_str("</Grantee><Permission>FULL_CONTROL</Permission></Grant>");
    if acl == ObjectAcl::PublicRead {
        output.push_str(
            "<Grant><Grantee xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"Group\">",
        );
        push_element(&mut output, "URI", ALL_USERS_GROUP)?;
        output.push_str("</Grantee><Permission>READ</Permission></Grant>");
    }
    output.push_str("</AccessControlList></AccessControlPolicy>");
    Ok(output)
}

#[derive(Debug)]
struct XmlNode {
    name: String,
    text: String,
    children: Vec<Self>,
}

fn parse_xml_document(input: &[u8]) -> Result<XmlNode, S3XmlError> {
    if input.is_empty() {
        return Err(S3XmlError::MalformedXml);
    }
    if input.len() > MAX_S3_XML_BODY_BYTES {
        return Err(S3XmlError::InputTooLarge);
    }

    let mut reader = Reader::from_reader(input);
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut stack = Vec::<XmlNode>::new();
    let mut root = None;
    let mut node_count = 0_usize;
    let mut declaration_seen = false;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|_| S3XmlError::MalformedXml)?;
        match event {
            Event::Start(element) => {
                validate_attributes(&reader, &element)?;
                if stack.len() == MAX_XML_DEPTH || root.is_some() {
                    return Err(S3XmlError::MalformedXml);
                }
                node_count += 1;
                if node_count > MAX_XML_NODES {
                    return Err(S3XmlError::MalformedXml);
                }
                stack.push(XmlNode {
                    name: decode_local_name(element.local_name().as_ref())?,
                    text: String::new(),
                    children: Vec::new(),
                });
            }
            Event::Empty(element) => {
                validate_attributes(&reader, &element)?;
                node_count += 1;
                if stack.len() == MAX_XML_DEPTH
                    || node_count > MAX_XML_NODES
                    || (stack.is_empty() && root.is_some())
                {
                    return Err(S3XmlError::MalformedXml);
                }
                let node = XmlNode {
                    name: decode_local_name(element.local_name().as_ref())?,
                    text: String::new(),
                    children: Vec::new(),
                };
                append_node(&mut stack, &mut root, node)?;
            }
            Event::End(_) => {
                let node = stack.pop().ok_or(S3XmlError::MalformedXml)?;
                append_node(&mut stack, &mut root, node)?;
            }
            Event::Text(text) => {
                let value = text.xml10_content().map_err(|_| S3XmlError::MalformedXml)?;
                append_text(&mut stack, &value)?;
            }
            Event::CData(text) => {
                let value = text.xml10_content().map_err(|_| S3XmlError::MalformedXml)?;
                append_text(&mut stack, &value)?;
            }
            Event::GeneralRef(reference) => {
                let character = if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|_| S3XmlError::MalformedXml)?
                {
                    character
                } else {
                    match reference
                        .decode()
                        .map_err(|_| S3XmlError::MalformedXml)?
                        .as_ref()
                    {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '\"',
                        "apos" => '\'',
                        _ => return Err(S3XmlError::MalformedXml),
                    }
                };
                if !is_xml_10_character(character) {
                    return Err(S3XmlError::MalformedXml);
                }
                append_text(&mut stack, &character.to_string())?;
            }
            Event::Decl(_) if !declaration_seen && stack.is_empty() && root.is_none() => {
                declaration_seen = true;
            }
            Event::Comment(_) => {}
            Event::DocType(_) | Event::PI(_) | Event::Decl(_) => {
                return Err(S3XmlError::MalformedXml);
            }
            Event::Eof => break,
        }
        buffer.clear();
    }
    if !stack.is_empty() {
        return Err(S3XmlError::MalformedXml);
    }
    root.ok_or(S3XmlError::MalformedXml)
}

fn validate_attributes(
    reader: &Reader<&[u8]>,
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<(), S3XmlError> {
    for attribute in element.attributes().with_checks(true) {
        attribute
            .map_err(|_| S3XmlError::MalformedXml)?
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|_| S3XmlError::MalformedXml)?;
    }
    Ok(())
}

fn decode_local_name(name: &[u8]) -> Result<String, S3XmlError> {
    std::str::from_utf8(name)
        .map(str::to_owned)
        .map_err(|_| S3XmlError::MalformedXml)
}

fn append_node(
    stack: &mut [XmlNode],
    root: &mut Option<XmlNode>,
    node: XmlNode,
) -> Result<(), S3XmlError> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
        Ok(())
    } else if root.is_none() {
        *root = Some(node);
        Ok(())
    } else {
        Err(S3XmlError::MalformedXml)
    }
}

fn append_text(stack: &mut [XmlNode], value: &str) -> Result<(), S3XmlError> {
    if !value.chars().all(is_xml_10_character) {
        return Err(S3XmlError::MalformedXml);
    }
    if let Some(node) = stack.last_mut() {
        node.text.push_str(value);
        Ok(())
    } else if value.trim().is_empty() {
        Ok(())
    } else {
        Err(S3XmlError::MalformedXml)
    }
}

fn xml_document_start(root_name: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><{root_name} xmlns=\"{S3_XML_NAMESPACE}\">")
}

fn push_element(output: &mut String, name: &str, value: &str) -> Result<(), S3XmlError> {
    write!(output, "<{name}>").expect("writing to String cannot fail");
    push_escaped_xml(output, value)?;
    write!(output, "</{name}>").expect("writing to String cannot fail");
    Ok(())
}

fn push_number_element(output: &mut String, name: &str, value: impl std::fmt::Display) {
    write!(output, "<{name}>{value}</{name}>").expect("writing to String cannot fail");
}

fn push_escaped_xml(output: &mut String, value: &str) -> Result<(), S3XmlError> {
    for character in value.chars() {
        if !is_xml_10_character(character) {
            return Err(S3XmlError::InvalidXmlCharacter);
        }
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    Ok(())
}

const fn is_xml_10_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || matches!(character as u32, 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_content_md5_against_the_exact_body() {
        assert_eq!(
            validate_content_md5(Some(b"kAFQmDzST7DWlj99KOF/cg=="), b"abc"),
            Ok(())
        );
        assert_eq!(
            validate_content_md5(Some(b"AAAAAAAAAAAAAAAAAAAAAA=="), b"abc"),
            Err(ContentMd5Error::BadDigest)
        );
        assert_eq!(ContentMd5Error::BadDigest.s3_code(), "BadDigest");
    }

    #[test]
    fn content_md5_rejects_missing_malformed_and_wrong_length_values() {
        assert_eq!(
            validate_content_md5(None, b"abc"),
            Err(ContentMd5Error::InvalidDigest)
        );
        assert_eq!(
            validate_content_md5(Some(b"not-base64!"), b"abc"),
            Err(ContentMd5Error::InvalidDigest)
        );
        assert_eq!(
            validate_content_md5(Some(b"AAAAAAAAAAAAAAAAAAAA"), b"abc"),
            Err(ContentMd5Error::InvalidDigest)
        );
        assert_eq!(ContentMd5Error::InvalidDigest.s3_code(), "InvalidDigest");
    }

    #[test]
    fn parses_delete_objects_and_unescapes_keys() {
        let request = parse_delete_objects_xml(
            br#"<?xml version="1.0"?><Delete xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Object><Key>a&amp;b&lt;c&gt;&#x2F;d</Key></Object><Object><Key><![CDATA[raw&key]]></Key></Object><Quiet>true</Quiet></Delete>"#,
        )
        .expect("valid delete request");
        assert_eq!(
            request,
            DeleteObjectsRequest {
                objects: vec![
                    DeleteObjectIdentifier {
                        key: "a&b<c>/d".to_owned(),
                    },
                    DeleteObjectIdentifier {
                        key: "raw&key".to_owned(),
                    },
                ],
                quiet: true,
            }
        );
    }

    #[test]
    fn delete_objects_enforces_key_version_and_count_limits() {
        assert_eq!(
            parse_delete_objects_xml(br#"<Delete><Object /></Delete>"#),
            Err(S3XmlError::MissingKey)
        );
        assert_eq!(
            parse_delete_objects_xml(
                br#"<Delete><Object><Key>a</Key><VersionId>1</VersionId></Object></Delete>"#,
            ),
            Err(S3XmlError::VersionIdNotSupported)
        );

        let mut xml = String::from("<Delete>");
        for index in 0..=MAX_DELETE_OBJECTS {
            write!(xml, "<Object><Key>{index}</Key></Object>").expect("String write");
        }
        xml.push_str("</Delete>");
        assert_eq!(
            parse_delete_objects_xml(xml.as_bytes()),
            Err(S3XmlError::TooManyObjects)
        );
    }

    #[test]
    fn parses_strictly_ordered_complete_multipart_upload() {
        let request = parse_complete_multipart_upload_xml(
            br#"<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>&quot;one&amp;two&quot;</ETag></Part><Part><PartNumber>10000</PartNumber><ETag>last</ETag></Part></CompleteMultipartUpload>"#,
        )
        .expect("valid complete request");
        assert_eq!(
            request.parts,
            vec![
                CompletedPart {
                    part_number: 1,
                    etag: "\"one&two\"".to_owned(),
                },
                CompletedPart {
                    part_number: 10_000,
                    etag: "last".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn complete_multipart_rejects_bad_numbers_missing_etags_and_order() {
        assert_eq!(
            parse_complete_multipart_upload_xml(
                br#"<CompleteMultipartUpload><Part><PartNumber>0</PartNumber><ETag>x</ETag></Part></CompleteMultipartUpload>"#,
            ),
            Err(S3XmlError::InvalidPartNumber)
        );
        assert_eq!(
            parse_complete_multipart_upload_xml(
                br#"<CompleteMultipartUpload><Part><PartNumber>1</PartNumber></Part></CompleteMultipartUpload>"#,
            ),
            Err(S3XmlError::MissingEtag)
        );
        assert_eq!(
            parse_complete_multipart_upload_xml(
                br#"<CompleteMultipartUpload><Part><PartNumber>2</PartNumber><ETag>x</ETag></Part><Part><PartNumber>2</PartNumber><ETag>y</ETag></Part></CompleteMultipartUpload>"#,
            ),
            Err(S3XmlError::InvalidPartOrder)
        );
    }

    #[test]
    fn serializes_delete_and_multipart_results_with_xml_escaping() {
        let deleted = delete_result_xml(&DeleteResult {
            deleted: vec![DeletedObject {
                key: "a<&>\"'".to_owned(),
            }],
            errors: vec![DeleteObjectError {
                key: "failed&key".to_owned(),
                code: "AccessDenied".to_owned(),
                message: "not <allowed>".to_owned(),
            }],
        })
        .expect("delete result");
        parse_xml_document(deleted.as_bytes()).expect("well-formed delete result XML");
        assert!(deleted.contains("<Key>a&lt;&amp;&gt;&quot;&apos;</Key>"));
        assert!(deleted.contains("<Error><Key>failed&amp;key</Key>"));

        let initiated =
            initiate_multipart_upload_result_xml("b&", "k<", "u>").expect("initiate result");
        parse_xml_document(initiated.as_bytes()).expect("well-formed initiate result XML");
        assert!(initiated.contains("<Bucket>b&amp;</Bucket><Key>k&lt;</Key>"));

        let completed = complete_multipart_upload_result_xml(
            "https://example.test/a?x=1&y=2",
            "bucket",
            "key",
            "\"abc\"",
        )
        .expect("complete result");
        parse_xml_document(completed.as_bytes()).expect("well-formed complete result XML");
        assert!(completed.contains("x=1&amp;y=2"));
        assert!(completed.contains("<ETag>&quot;abc&quot;</ETag>"));
    }

    #[test]
    fn serializes_list_parts_metadata_and_acl_variants() {
        let xml = list_parts_result_xml(&ListPartsResult {
            bucket: "bucket".to_owned(),
            key: "key".to_owned(),
            upload_id: "upload".to_owned(),
            part_number_marker: 1,
            next_part_number_marker: 2,
            max_parts: 1000,
            is_truncated: true,
            parts: vec![ListedPart {
                part_number: 2,
                last_modified: "2026-07-19T00:00:00.000Z".to_owned(),
                etag: "\"abc\"".to_owned(),
                size: 42,
            }],
        })
        .expect("list parts result");
        parse_xml_document(xml.as_bytes()).expect("well-formed list parts result XML");
        assert!(xml.contains("<PartNumberMarker>1</PartNumberMarker>"));
        assert!(xml.contains("<NextPartNumberMarker>2</NextPartNumberMarker>"));
        assert!(xml.contains("<MaxParts>1000</MaxParts><IsTruncated>true</IsTruncated>"));
        assert!(xml.contains("<Part><PartNumber>2</PartNumber>"));

        let private =
            get_object_acl_xml("owner&1", "owner", ObjectAcl::Private).expect("private ACL");
        parse_xml_document(private.as_bytes()).expect("well-formed private ACL XML");
        assert!(private.contains("<ID>owner&amp;1</ID>"));
        assert!(!private.contains(ALL_USERS_GROUP));
        let public =
            get_object_acl_xml("owner", "owner", ObjectAcl::PublicRead).expect("public ACL");
        parse_xml_document(public.as_bytes()).expect("well-formed public ACL XML");
        assert!(public.contains(ALL_USERS_GROUP));
        assert!(public.contains("<Permission>READ</Permission>"));
    }

    #[test]
    fn rejects_doctype_unknown_entities_oversized_input_and_illegal_output() {
        assert_eq!(
            parse_delete_objects_xml(
                br#"<!DOCTYPE Delete [<!ENTITY x "expanded">]><Delete><Object><Key>&x;</Key></Object></Delete>"#,
            ),
            Err(S3XmlError::MalformedXml)
        );
        assert_eq!(
            parse_delete_objects_xml(br#"<Delete><Object><Key>&unknown;</Key></Object></Delete>"#,),
            Err(S3XmlError::MalformedXml)
        );
        assert_eq!(
            parse_delete_objects_xml(&vec![b' '; MAX_S3_XML_BODY_BYTES + 1]),
            Err(S3XmlError::InputTooLarge)
        );
        assert_eq!(
            initiate_multipart_upload_result_xml("bucket", "bad\u{1}key", "upload"),
            Err(S3XmlError::InvalidXmlCharacter)
        );
    }
}
