// S3 service and Bucket protocol helpers.

const DEFAULT_S3_REGION: &str = "us-east-1";
const S3_XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";
const MAX_S3_CREATE_BUCKET_CONFIGURATION_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S3CreateBucketConfigurationError {
    MalformedXml,
    InvalidLocationConstraint,
}

fn validate_s3_bucket_name(name: &str) -> Result<(), ()> {
    let bytes = name.as_bytes();
    let valid_edge = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    if !(3..=63).contains(&bytes.len())
        || !name.is_ascii()
        || !valid_edge(bytes[0])
        || !valid_edge(bytes[bytes.len() - 1])
        || bytes.iter().any(|byte| {
            !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-'))
        })
        || name.contains("..")
        || name.parse::<std::net::IpAddr>().is_ok()
        || ["xn--", "sthree-", "amzn_s3_demo_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        || ["-s3alias", "--ol-s3", ".mrap", "--x-s3", "--table-s3"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
    {
        Err(())
    } else {
        Ok(())
    }
}

fn s3_list_buckets_xml(
    owner_id: &str,
    owner_display_name: &str,
    buckets: &[Bucket],
) -> Result<String, time::error::Format> {
    let mut xml = String::with_capacity(320 + buckets.len().saturating_mul(160));
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    xml.push_str("<ListAllMyBucketsResult xmlns=\"");
    xml.push_str(S3_XML_NAMESPACE);
    xml.push_str("\"><Owner><ID>");
    xml.push_str(&escape_s3_xml(owner_id));
    xml.push_str("</ID><DisplayName>");
    xml.push_str(&escape_s3_xml(owner_display_name));
    xml.push_str("</DisplayName></Owner><Buckets>");
    for bucket in buckets {
        let created_at = bucket
            .created_at()
            .to_offset(time::UtcOffset::UTC)
            .format(&time::format_description::well_known::Rfc3339)?;
        xml.push_str("<Bucket><Name>");
        xml.push_str(&escape_s3_xml(bucket.name()));
        xml.push_str("</Name><CreationDate>");
        xml.push_str(&created_at);
        xml.push_str("</CreationDate></Bucket>");
    }
    xml.push_str("</Buckets></ListAllMyBucketsResult>");
    Ok(xml)
}

fn s3_bucket_location_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><LocationConstraint xmlns=\"{S3_XML_NAMESPACE}\"></LocationConstraint>"
    )
}

fn s3_bucket_region_response(status: StatusCode, request_id: &str) -> Response {
    let mut response = s3_empty_response(status, request_id);
    response.headers_mut().insert(
        HeaderName::from_static("x-amz-bucket-region"),
        HeaderValue::from_static(DEFAULT_S3_REGION),
    );
    response
}

fn validate_s3_create_bucket_configuration(
    input: &[u8],
) -> Result<(), S3CreateBucketConfigurationError> {
    if input.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    if input.len() > MAX_S3_CREATE_BUCKET_CONFIGURATION_BYTES {
        return Err(S3CreateBucketConfigurationError::MalformedXml);
    }

    let mut reader = Reader::from_reader(input);
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut stack = Vec::<String>::new();
    let mut root_seen = false;
    let mut declaration_seen = false;
    let mut location = None::<String>;

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|_| S3CreateBucketConfigurationError::MalformedXml)?;
        match event {
            Event::Start(element) => {
                let name = std::str::from_utf8(element.local_name().as_ref())
                    .map_err(|_| S3CreateBucketConfigurationError::MalformedXml)?
                    .to_owned();
                validate_s3_create_bucket_attributes(&reader, &element, stack.is_empty())?;
                match stack.as_slice() {
                    [] if !root_seen && name == "CreateBucketConfiguration" => {
                        root_seen = true;
                    }
                    [root]
                        if root == "CreateBucketConfiguration"
                            && name == "LocationConstraint"
                            && location.is_none() =>
                    {
                        location = Some(String::new());
                    }
                    _ => return Err(S3CreateBucketConfigurationError::MalformedXml),
                }
                stack.push(name);
            }
            Event::Empty(element) => {
                let name = std::str::from_utf8(element.local_name().as_ref())
                    .map_err(|_| S3CreateBucketConfigurationError::MalformedXml)?
                    .to_owned();
                validate_s3_create_bucket_attributes(&reader, &element, stack.is_empty())?;
                match stack.as_slice() {
                    [] if !root_seen && name == "CreateBucketConfiguration" => {
                        root_seen = true;
                    }
                    [root]
                        if root == "CreateBucketConfiguration"
                            && name == "LocationConstraint"
                            && location.is_none() =>
                    {
                        location = Some(String::new());
                    }
                    _ => return Err(S3CreateBucketConfigurationError::MalformedXml),
                }
            }
            Event::End(_) => {
                stack
                    .pop()
                    .ok_or(S3CreateBucketConfigurationError::MalformedXml)?;
            }
            Event::Text(text) => {
                let value = text
                    .xml10_content()
                    .map_err(|_| S3CreateBucketConfigurationError::MalformedXml)?;
                append_s3_create_bucket_text(&stack, &mut location, &value)?;
            }
            Event::CData(text) => {
                let value = text
                    .xml10_content()
                    .map_err(|_| S3CreateBucketConfigurationError::MalformedXml)?;
                append_s3_create_bucket_text(&stack, &mut location, &value)?;
            }
            Event::Decl(_) if !declaration_seen && stack.is_empty() && !root_seen => {
                declaration_seen = true;
            }
            Event::Comment(_) => {}
            Event::DocType(_) | Event::PI(_) | Event::Decl(_) | Event::GeneralRef(_) => {
                return Err(S3CreateBucketConfigurationError::MalformedXml);
            }
            Event::Eof => break,
        }
        buffer.clear();
    }

    if !root_seen || !stack.is_empty() {
        return Err(S3CreateBucketConfigurationError::MalformedXml);
    }
    match location {
        None => Ok(()),
        Some(_) => Err(S3CreateBucketConfigurationError::InvalidLocationConstraint),
    }
}

fn validate_s3_create_bucket_attributes(
    reader: &Reader<&[u8]>,
    element: &quick_xml::events::BytesStart<'_>,
    is_root: bool,
) -> Result<(), S3CreateBucketConfigurationError> {
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| S3CreateBucketConfigurationError::MalformedXml)?;
        let key = std::str::from_utf8(attribute.key.as_ref())
            .map_err(|_| S3CreateBucketConfigurationError::MalformedXml)?;
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|_| S3CreateBucketConfigurationError::MalformedXml)?;
        if !is_root || key != "xmlns" || value.as_ref() != S3_XML_NAMESPACE {
            return Err(S3CreateBucketConfigurationError::MalformedXml);
        }
    }
    Ok(())
}

fn append_s3_create_bucket_text(
    stack: &[String],
    location: &mut Option<String>,
    value: &str,
) -> Result<(), S3CreateBucketConfigurationError> {
    match stack {
        [root, current]
            if root == "CreateBucketConfiguration" && current == "LocationConstraint" =>
        {
            location
                .as_mut()
                .ok_or(S3CreateBucketConfigurationError::MalformedXml)?
                .push_str(value);
            Ok(())
        }
        _ if value.trim().is_empty() => Ok(()),
        _ => Err(S3CreateBucketConfigurationError::MalformedXml),
    }
}
