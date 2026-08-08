// Pure S3 Bucket CORS protocol codecs. Persistence and HTTP routing are wired separately.

pub(crate) mod s3_cors_protocol {
    use std::fmt::Write as _;

    use mediahub_core::{S3CorsConfiguration, S3CorsError, S3CorsMethod, S3CorsRule};

    const S3_CORS_XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";
    pub(crate) const MAX_S3_CORS_DOCUMENT_BYTES: usize = 64 * 1024;
    const MAX_S3_CORS_XML_DEPTH: usize = 3;
    const MAX_S3_CORS_XML_NODES: usize = 40_001;

    #[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
    pub(crate) enum S3CorsProtocolError {
        #[error("the CORS XML document exceeds the 64 KiB S3 limit")]
        InputTooLarge,
        #[error("the Content-MD5 header is required")]
        MissingContentMd5,
        #[error("the Content-MD5 header is not a valid Base64-encoded MD5 digest")]
        InvalidDigest,
        #[error("the Content-MD5 header does not match the request body")]
        BadDigest,
        #[error("the CORS XML document is malformed")]
        MalformedXml,
        #[error("the CORS configuration is invalid: {0}")]
        InvalidConfiguration(S3CorsError),
    }

    impl S3CorsProtocolError {
        #[cfg(test)]
        pub(crate) const fn s3_code(&self) -> &'static str {
            match self {
                Self::InputTooLarge => "EntityTooLarge",
                Self::MissingContentMd5 => "MissingContentMD5",
                Self::InvalidDigest => "InvalidDigest",
                Self::BadDigest => "BadDigest",
                Self::MalformedXml | Self::InvalidConfiguration(_) => "MalformedXML",
            }
        }
    }

    /// Applies the protocol body limit and required Content-MD5 check before XML parsing.
    pub(crate) fn parse_s3_cors_put_xml(
        content_md5: Option<&[u8]>,
        input: &[u8],
    ) -> Result<S3CorsConfiguration, S3CorsProtocolError> {
        if input.len() > MAX_S3_CORS_DOCUMENT_BYTES {
            return Err(S3CorsProtocolError::InputTooLarge);
        }
        validate_s3_cors_content_md5(content_md5, input)?;
        parse_s3_cors_configuration_xml(input)
    }

    pub(crate) fn validate_s3_cors_content_md5(
        content_md5: Option<&[u8]>,
        input: &[u8],
    ) -> Result<(), S3CorsProtocolError> {
        let content_md5 = content_md5.ok_or(S3CorsProtocolError::MissingContentMd5)?;
        crate::s3_xml::validate_content_md5(Some(content_md5), input).map_err(|error| match error {
            crate::s3_xml::ContentMd5Error::InvalidDigest => S3CorsProtocolError::InvalidDigest,
            crate::s3_xml::ContentMd5Error::BadDigest => S3CorsProtocolError::BadDigest,
        })
    }

    pub(crate) fn parse_s3_cors_configuration_xml(
        input: &[u8],
    ) -> Result<S3CorsConfiguration, S3CorsProtocolError> {
        if input.len() > MAX_S3_CORS_DOCUMENT_BYTES {
            return Err(S3CorsProtocolError::InputTooLarge);
        }
        let root = parse_s3_cors_xml_document(input)?;
        validate_s3_cors_root(&root)?;
        let rules = root
            .children
            .iter()
            .map(parse_s3_cors_rule)
            .collect::<Result<Vec<_>, _>>()?;
        S3CorsConfiguration::new(rules).map_err(S3CorsProtocolError::InvalidConfiguration)
    }

    pub(crate) fn render_s3_cors_configuration_xml(
        configuration: &S3CorsConfiguration,
    ) -> Result<String, S3CorsProtocolError> {
        configuration
            .validate()
            .map_err(S3CorsProtocolError::InvalidConfiguration)?;
        let mut output = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><CORSConfiguration xmlns=\"{S3_CORS_XML_NAMESPACE}\">"
        );
        for rule in configuration.rules() {
            output.push_str("<CORSRule>");
            for header in rule.allowed_headers() {
                push_s3_cors_xml_element(&mut output, "AllowedHeader", header);
            }
            for method in rule.allowed_methods() {
                push_s3_cors_xml_element(&mut output, "AllowedMethod", method.as_str());
            }
            for origin in rule.allowed_origins() {
                push_s3_cors_xml_element(&mut output, "AllowedOrigin", origin);
            }
            for header in rule.expose_headers() {
                push_s3_cors_xml_element(&mut output, "ExposeHeader", header);
            }
            if let Some(id) = rule.id() {
                push_s3_cors_xml_element(&mut output, "ID", id);
            }
            if let Some(max_age_seconds) = rule.max_age_seconds() {
                write!(output, "<MaxAgeSeconds>{max_age_seconds}</MaxAgeSeconds>")
                    .expect("writing to String cannot fail");
            }
            output.push_str("</CORSRule>");
        }
        output.push_str("</CORSConfiguration>");
        if output.len() > MAX_S3_CORS_DOCUMENT_BYTES {
            return Err(S3CorsProtocolError::InputTooLarge);
        }
        Ok(output)
    }

    #[derive(Debug)]
    struct S3CorsXmlElement {
        name: String,
        attributes: Vec<(String, String)>,
        text: String,
        children: Vec<Self>,
    }

    fn parse_s3_cors_xml_document(input: &[u8]) -> Result<S3CorsXmlElement, S3CorsProtocolError> {
        if input.is_empty() {
            return Err(S3CorsProtocolError::MalformedXml);
        }
        let mut reader = quick_xml::Reader::from_reader(input);
        reader.config_mut().check_end_names = true;
        let mut buffer = Vec::new();
        let mut stack = Vec::<S3CorsXmlElement>::new();
        let mut root = None;
        let mut node_count = 0_usize;
        let mut declaration_allowed = true;
        let mut declaration_seen = false;

        loop {
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|_| S3CorsProtocolError::MalformedXml)?;
            match event {
                quick_xml::events::Event::Start(element) => {
                    declaration_allowed = false;
                    node_count = node_count.saturating_add(1);
                    if stack.len() >= MAX_S3_CORS_XML_DEPTH || node_count > MAX_S3_CORS_XML_NODES {
                        return Err(S3CorsProtocolError::MalformedXml);
                    }
                    stack.push(new_s3_cors_xml_element(&reader, &element)?);
                }
                quick_xml::events::Event::Empty(element) => {
                    declaration_allowed = false;
                    node_count = node_count.saturating_add(1);
                    if stack.len() >= MAX_S3_CORS_XML_DEPTH || node_count > MAX_S3_CORS_XML_NODES {
                        return Err(S3CorsProtocolError::MalformedXml);
                    }
                    let element = new_s3_cors_xml_element(&reader, &element)?;
                    attach_s3_cors_xml_element(&mut stack, &mut root, element)?;
                }
                quick_xml::events::Event::End(_) => {
                    declaration_allowed = false;
                    let element = stack.pop().ok_or(S3CorsProtocolError::MalformedXml)?;
                    attach_s3_cors_xml_element(&mut stack, &mut root, element)?;
                }
                quick_xml::events::Event::Text(text) => {
                    let value = text
                        .xml10_content()
                        .map_err(|_| S3CorsProtocolError::MalformedXml)?;
                    if !value.is_empty() {
                        declaration_allowed = false;
                    }
                    append_s3_cors_xml_text(&mut stack, &value)?;
                }
                quick_xml::events::Event::CData(text) => {
                    declaration_allowed = false;
                    let value = text
                        .xml10_content()
                        .map_err(|_| S3CorsProtocolError::MalformedXml)?;
                    append_s3_cors_xml_text(&mut stack, &value)?;
                }
                quick_xml::events::Event::Decl(declaration)
                    if declaration_allowed
                        && !declaration_seen
                        && stack.is_empty()
                        && root.is_none() =>
                {
                    validate_s3_cors_xml_declaration(&declaration)?;
                    declaration_seen = true;
                    declaration_allowed = false;
                }
                quick_xml::events::Event::Comment(_) => {
                    declaration_allowed = false;
                }
                quick_xml::events::Event::GeneralRef(reference) => {
                    declaration_allowed = false;
                    let character = resolve_s3_cors_xml_reference(&reference)?;
                    append_s3_cors_xml_text(&mut stack, &character.to_string())?;
                }
                quick_xml::events::Event::DocType(_)
                | quick_xml::events::Event::PI(_)
                | quick_xml::events::Event::Decl(_) => {
                    return Err(S3CorsProtocolError::MalformedXml);
                }
                quick_xml::events::Event::Eof => break,
            }
            buffer.clear();
        }
        if !stack.is_empty() {
            return Err(S3CorsProtocolError::MalformedXml);
        }
        root.ok_or(S3CorsProtocolError::MalformedXml)
    }

    fn validate_s3_cors_xml_declaration(
        declaration: &quick_xml::events::BytesDecl<'_>,
    ) -> Result<(), S3CorsProtocolError> {
        if declaration
            .version()
            .map_err(|_| S3CorsProtocolError::MalformedXml)?
            .as_ref()
            != b"1.0"
        {
            return Err(S3CorsProtocolError::MalformedXml);
        }
        if let Some(encoding) = declaration.encoding() {
            let encoding = encoding.map_err(|_| S3CorsProtocolError::MalformedXml)?;
            if !encoding.as_ref().eq_ignore_ascii_case(b"UTF-8") {
                return Err(S3CorsProtocolError::MalformedXml);
            }
        }
        Ok(())
    }

    fn new_s3_cors_xml_element(
        reader: &quick_xml::Reader<&[u8]>,
        element: &quick_xml::events::BytesStart<'_>,
    ) -> Result<S3CorsXmlElement, S3CorsProtocolError> {
        let qualified_name = element.name();
        let name = std::str::from_utf8(qualified_name.as_ref())
            .map_err(|_| S3CorsProtocolError::MalformedXml)?
            .to_owned();
        let mut attributes = Vec::new();
        for attribute in element.attributes().with_checks(true) {
            let attribute = attribute.map_err(|_| S3CorsProtocolError::MalformedXml)?;
            let key = std::str::from_utf8(attribute.key.as_ref())
                .map_err(|_| S3CorsProtocolError::MalformedXml)?
                .to_owned();
            let value = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
                .map_err(|_| S3CorsProtocolError::MalformedXml)?
                .into_owned();
            attributes.push((key, value));
        }
        Ok(S3CorsXmlElement {
            name,
            attributes,
            text: String::new(),
            children: Vec::new(),
        })
    }

    fn attach_s3_cors_xml_element(
        stack: &mut [S3CorsXmlElement],
        root: &mut Option<S3CorsXmlElement>,
        element: S3CorsXmlElement,
    ) -> Result<(), S3CorsProtocolError> {
        if let Some(parent) = stack.last_mut() {
            parent.children.push(element);
        } else if root.replace(element).is_some() {
            return Err(S3CorsProtocolError::MalformedXml);
        }
        Ok(())
    }

    fn append_s3_cors_xml_text(
        stack: &mut [S3CorsXmlElement],
        value: &str,
    ) -> Result<(), S3CorsProtocolError> {
        if let Some(element) = stack.last_mut() {
            element.text.push_str(value);
            Ok(())
        } else if value.trim().is_empty() {
            Ok(())
        } else {
            Err(S3CorsProtocolError::MalformedXml)
        }
    }

    fn resolve_s3_cors_xml_reference(
        reference: &quick_xml::events::BytesRef<'_>,
    ) -> Result<char, S3CorsProtocolError> {
        let character = if let Some(character) = reference
            .resolve_char_ref()
            .map_err(|_| S3CorsProtocolError::MalformedXml)?
        {
            character
        } else {
            match reference
                .decode()
                .map_err(|_| S3CorsProtocolError::MalformedXml)?
                .as_ref()
            {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => return Err(S3CorsProtocolError::MalformedXml),
            }
        };
        if is_s3_cors_xml_character(character) {
            Ok(character)
        } else {
            Err(S3CorsProtocolError::MalformedXml)
        }
    }

    fn validate_s3_cors_root(root: &S3CorsXmlElement) -> Result<(), S3CorsProtocolError> {
        if root.name != "CORSConfiguration" || !root.text.trim().is_empty() {
            return Err(S3CorsProtocolError::MalformedXml);
        }
        match root.attributes.as_slice() {
            [] => {}
            [(name, value)] if name == "xmlns" && value == S3_CORS_XML_NAMESPACE => {}
            _ => return Err(S3CorsProtocolError::MalformedXml),
        }
        if root.children.iter().any(|child| child.name != "CORSRule") {
            return Err(S3CorsProtocolError::MalformedXml);
        }
        Ok(())
    }

    fn parse_s3_cors_rule(element: &S3CorsXmlElement) -> Result<S3CorsRule, S3CorsProtocolError> {
        validate_s3_cors_container(element)?;
        let mut id = None;
        let mut allowed_methods = Vec::new();
        let mut allowed_origins = Vec::new();
        let mut allowed_headers = Vec::new();
        let mut expose_headers = Vec::new();
        let mut max_age_seconds = None;

        for child in &element.children {
            let text = required_s3_cors_leaf_text(child)?;
            match child.name.as_str() {
                "ID" if id.is_none() => id = Some(text.to_owned()),
                "AllowedMethod" => allowed_methods.push(
                    text.parse::<S3CorsMethod>()
                        .map_err(S3CorsProtocolError::InvalidConfiguration)?,
                ),
                "AllowedOrigin" => allowed_origins.push(text.to_owned()),
                "AllowedHeader" => allowed_headers.push(text.to_owned()),
                "ExposeHeader" => expose_headers.push(text.to_owned()),
                "MaxAgeSeconds" if max_age_seconds.is_none() => {
                    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err(S3CorsProtocolError::MalformedXml);
                    }
                    max_age_seconds = Some(
                        text.parse::<u32>()
                            .map_err(|_| S3CorsProtocolError::MalformedXml)?,
                    );
                }
                _ => return Err(S3CorsProtocolError::MalformedXml),
            }
        }
        S3CorsRule::new(
            id,
            allowed_methods,
            allowed_origins,
            allowed_headers,
            expose_headers,
            max_age_seconds,
        )
        .map_err(S3CorsProtocolError::InvalidConfiguration)
    }

    fn validate_s3_cors_container(element: &S3CorsXmlElement) -> Result<(), S3CorsProtocolError> {
        if element.attributes.is_empty() && element.text.trim().is_empty() {
            Ok(())
        } else {
            Err(S3CorsProtocolError::MalformedXml)
        }
    }

    fn required_s3_cors_leaf_text(element: &S3CorsXmlElement) -> Result<&str, S3CorsProtocolError> {
        if element.attributes.is_empty() && element.children.is_empty() {
            Ok(element.text.as_str())
        } else {
            Err(S3CorsProtocolError::MalformedXml)
        }
    }

    fn push_s3_cors_xml_element(output: &mut String, name: &str, value: &str) {
        write!(output, "<{name}>").expect("writing to String cannot fail");
        for character in value.chars() {
            match character {
                '&' => output.push_str("&amp;"),
                '<' => output.push_str("&lt;"),
                '>' => output.push_str("&gt;"),
                '"' => output.push_str("&quot;"),
                '\'' => output.push_str("&apos;"),
                character => output.push(character),
            }
        }
        write!(output, "</{name}>").expect("writing to String cannot fail");
    }

    const fn is_s3_cors_xml_character(character: char) -> bool {
        matches!(
            character,
            '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
        )
    }

    #[cfg(test)]
    mod s3_cors_protocol_tests {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
        use md5::{Digest as _, Md5};

        use super::*;

        const VALID_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<CORSConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <CORSRule>
    <ID>upload&amp;preview</ID>
    <AllowedOrigin>https://*.example.com</AllowedOrigin>
    <AllowedMethod>PUT</AllowedMethod>
    <AllowedMethod>GET</AllowedMethod>
    <AllowedHeader>x-amz-*</AllowedHeader>
    <ExposeHeader>etag</ExposeHeader>
    <MaxAgeSeconds>3600</MaxAgeSeconds>
  </CORSRule>
</CORSConfiguration>"#;

        #[test]
        fn parses_aws_cors_shape_and_preserves_rule_value_order() {
            let configuration = parse_s3_cors_configuration_xml(VALID_XML).expect("valid CORS XML");
            let rule = &configuration.rules()[0];

            assert_eq!(rule.id(), Some("upload&preview"));
            assert_eq!(
                rule.allowed_methods(),
                &[S3CorsMethod::Put, S3CorsMethod::Get]
            );
            assert_eq!(rule.allowed_origins(), &["https://*.example.com"]);
            assert_eq!(rule.allowed_headers(), &["x-amz-*"]);
            assert_eq!(rule.expose_headers(), &["etag"]);
            assert_eq!(rule.max_age_seconds(), Some(3_600));
        }

        #[test]
        fn rendered_xml_is_namespaced_escaped_and_round_trips() {
            let configuration = parse_s3_cors_configuration_xml(VALID_XML).expect("valid CORS XML");
            let xml = render_s3_cors_configuration_xml(&configuration).expect("rendered XML");

            assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
            assert!(xml.contains(&format!("xmlns=\"{S3_CORS_XML_NAMESPACE}\"")));
            assert!(xml.contains("<ID>upload&amp;preview</ID>"));
            assert_eq!(
                parse_s3_cors_configuration_xml(xml.as_bytes()),
                Ok(configuration)
            );
        }

        #[test]
        fn put_body_requires_a_well_formed_matching_content_md5() {
            let digest = BASE64_STANDARD.encode(Md5::digest(VALID_XML));
            assert!(parse_s3_cors_put_xml(Some(digest.as_bytes()), VALID_XML).is_ok());
            assert_eq!(
                parse_s3_cors_put_xml(None, VALID_XML),
                Err(S3CorsProtocolError::MissingContentMd5)
            );
            assert_eq!(
                parse_s3_cors_put_xml(Some(b"not-base64"), VALID_XML),
                Err(S3CorsProtocolError::InvalidDigest)
            );
            assert_eq!(
                parse_s3_cors_put_xml(Some(b"AAAAAAAAAAAAAAAAAAAAAA=="), VALID_XML),
                Err(S3CorsProtocolError::BadDigest)
            );
        }

        #[test]
        fn protocol_errors_have_stable_s3_error_codes() {
            assert_eq!(
                S3CorsProtocolError::InputTooLarge.s3_code(),
                "EntityTooLarge"
            );
            assert_eq!(
                S3CorsProtocolError::MissingContentMd5.s3_code(),
                "MissingContentMD5"
            );
            assert_eq!(
                S3CorsProtocolError::InvalidDigest.s3_code(),
                "InvalidDigest"
            );
            assert_eq!(S3CorsProtocolError::BadDigest.s3_code(), "BadDigest");
            assert_eq!(S3CorsProtocolError::MalformedXml.s3_code(), "MalformedXML");
            assert_eq!(
                S3CorsProtocolError::InvalidConfiguration(S3CorsError::MissingAllowedOrigin)
                    .s3_code(),
                "MalformedXML"
            );
        }

        #[test]
        fn unknown_elements_attributes_and_duplicate_singletons_fail_closed() {
            for xml in [
                "<CORSConfiguration><Unknown /></CORSConfiguration>",
                "<CORSConfiguration extra=\"1\"><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>",
                "<CORSConfiguration><CORSRule extra=\"1\"><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>",
                "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod><ID>one</ID><ID>two</ID></CORSRule></CORSConfiguration>",
                "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod><MaxAgeSeconds>1</MaxAgeSeconds><MaxAgeSeconds>2</MaxAgeSeconds></CORSRule></CORSConfiguration>",
                "<x:CORSConfiguration xmlns:x=\"http://s3.amazonaws.com/doc/2006-03-01/\"><x:CORSRule /></x:CORSConfiguration>",
            ] {
                assert_eq!(
                    parse_s3_cors_configuration_xml(xml.as_bytes()),
                    Err(S3CorsProtocolError::MalformedXml),
                    "{xml}"
                );
            }
        }

        #[test]
        fn empty_or_duplicate_rules_and_invalid_values_fail_closed() {
            assert_eq!(
                parse_s3_cors_configuration_xml(b"<CORSConfiguration />"),
                Err(S3CorsProtocolError::InvalidConfiguration(
                    S3CorsError::EmptyConfiguration
                ))
            );
            assert_eq!(
                parse_s3_cors_configuration_xml(
                    b"<CORSConfiguration><CORSRule /></CORSConfiguration>"
                ),
                Err(S3CorsProtocolError::InvalidConfiguration(
                    S3CorsError::MissingAllowedMethod
                ))
            );
            assert_eq!(
            parse_s3_cors_configuration_xml(
                b"<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>"
            ),
            Err(S3CorsProtocolError::InvalidConfiguration(
                S3CorsError::DuplicateRule
            ))
        );
            for (xml, error) in [
                (
                    "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>OPTIONS</AllowedMethod></CORSRule></CORSConfiguration>",
                    S3CorsError::InvalidAllowedMethod,
                ),
                (
                    "<CORSConfiguration><CORSRule><AllowedOrigin>https://*.*.example.com</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>",
                    S3CorsError::InvalidAllowedOrigin,
                ),
                (
                    "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>",
                    S3CorsError::DuplicateAllowedOrigin,
                ),
                (
                    "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>",
                    S3CorsError::DuplicateAllowedMethod,
                ),
                (
                    "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod><AllowedHeader>x-*-*</AllowedHeader></CORSRule></CORSConfiguration>",
                    S3CorsError::InvalidAllowedHeader,
                ),
                (
                    "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod><ExposeHeader>x-amz-*</ExposeHeader></CORSRule></CORSConfiguration>",
                    S3CorsError::InvalidExposeHeader,
                ),
            ] {
                assert_eq!(
                    parse_s3_cors_configuration_xml(xml.as_bytes()),
                    Err(S3CorsProtocolError::InvalidConfiguration(error)),
                    "{xml}"
                );
            }
        }

        #[test]
        fn limits_depth_and_dangerous_xml_constructs() {
            let oversized = vec![b' '; MAX_S3_CORS_DOCUMENT_BYTES + 1];
            assert_eq!(
                parse_s3_cors_configuration_xml(&oversized),
                Err(S3CorsProtocolError::InputTooLarge)
            );
            for xml in [
                "<!DOCTYPE CORSConfiguration [<!ENTITY x \"*\">]><CORSConfiguration><CORSRule><AllowedOrigin>&x;</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>",
                "<CORSConfiguration><CORSRule><AllowedOrigin><Nested>*</Nested></AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>",
                "<CORSConfiguration><CORSRule><AllowedOrigin>&unknown;</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule></CORSConfiguration>",
                "<CORSConfiguration /><CORSConfiguration />",
                "<?xml version=\"1.1\"?><CORSConfiguration />",
                "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><CORSConfiguration />",
            ] {
                assert_eq!(
                    parse_s3_cors_configuration_xml(xml.as_bytes()),
                    Err(S3CorsProtocolError::MalformedXml),
                    "{xml}"
                );
            }
        }

        #[test]
        fn enforces_100_rule_limit_and_unsigned_decimal_max_age() {
            let mut xml = String::from("<CORSConfiguration>");
            for _ in 0..=mediahub_core::MAX_S3_CORS_RULES {
                xml.push_str(
                "<CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod></CORSRule>",
            );
            }
            xml.push_str("</CORSConfiguration>");
            assert_eq!(
                parse_s3_cors_configuration_xml(xml.as_bytes()),
                Err(S3CorsProtocolError::InvalidConfiguration(
                    S3CorsError::TooManyRules
                ))
            );
            for value in ["", "-1", "+1", "4294967296"] {
                let xml = format!(
                    "<CORSConfiguration><CORSRule><AllowedOrigin>*</AllowedOrigin><AllowedMethod>GET</AllowedMethod><MaxAgeSeconds>{value}</MaxAgeSeconds></CORSRule></CORSConfiguration>"
                );
                assert_eq!(
                    parse_s3_cors_configuration_xml(xml.as_bytes()),
                    Err(S3CorsProtocolError::MalformedXml)
                );
            }
        }
    }
}
