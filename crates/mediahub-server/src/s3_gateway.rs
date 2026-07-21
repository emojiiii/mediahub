use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime},
};

use aws_credential_types::Credentials;
use aws_sigv4::{
    http_request::{
        PayloadChecksumKind, PercentEncodingMode, SignableBody, SignableRequest, SignatureLocation,
        SigningSettings, UriPathNormalizationMode, sign,
    },
    sign::v4,
};
use chrono::{DateTime, NaiveDateTime, Utc};
use http::{HeaderMap, Method, Uri};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

const ALGORITHM: &str = "AWS4-HMAC-SHA256";
const TERMINATOR: &str = "aws4_request";
const SERVICE: &str = "s3";
const MAX_HEADER_SKEW: Duration = Duration::from_secs(5 * 60);
const MAX_PRESIGN_EXPIRY: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthLocation {
    Header,
    Query,
}

#[derive(Clone, Debug)]
enum PayloadMode {
    Calculate,
    Unsigned,
    Precomputed(String),
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedSigV4 {
    access_key_id: String,
    region: String,
    signing_time: SystemTime,
    location: AuthLocation,
    expires_in: Option<Duration>,
    method: String,
    signing_uri: String,
    signed_headers: Vec<(String, String)>,
    payload_mode: PayloadMode,
    supplied_signature: String,
}

impl ParsedSigV4 {
    pub(crate) fn parse(
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        now: SystemTime,
    ) -> Result<Self, SigV4Error> {
        let parsed = if let Some(authorization) = headers.get(http::header::AUTHORIZATION) {
            Self::parse_header(
                method,
                uri,
                headers,
                authorization
                    .to_str()
                    .map_err(|_| SigV4Error::InvalidRequest)?,
            )?
        } else {
            Self::parse_query(method, uri, headers)?
        };
        parsed.validate_time(now)?;
        Ok(parsed)
    }

    pub(crate) fn access_key_id(&self) -> &str {
        &self.access_key_id
    }

    pub(crate) fn verify(&self, secret: &str, body: &[u8]) -> Result<(), SigV4Error> {
        let body_hash;
        let signable_body = match &self.payload_mode {
            PayloadMode::Calculate => SignableBody::Bytes(body),
            PayloadMode::Unsigned => SignableBody::UnsignedPayload,
            PayloadMode::Precomputed(expected) => {
                body_hash = hex::encode(Sha256::digest(body));
                if !constant_time_eq(&body_hash, expected) {
                    return Err(SigV4Error::PayloadHashMismatch);
                }
                SignableBody::Precomputed(expected.clone())
            }
        };
        self.verify_signature(secret, signable_body)
    }

    /// Verifies an upload request without buffering its payload. A streaming
    /// request must either opt into `UNSIGNED-PAYLOAD` or provide the payload
    /// digest in `x-amz-content-sha256` so the canonical request is known
    /// before the body is accepted.
    pub(crate) fn verify_streaming_signature(&self, secret: &str) -> Result<(), SigV4Error> {
        let signable_body = match &self.payload_mode {
            PayloadMode::Unsigned => SignableBody::UnsignedPayload,
            PayloadMode::Precomputed(expected) => SignableBody::Precomputed(expected.clone()),
            PayloadMode::Calculate => return Err(SigV4Error::StreamingPayloadHashRequired),
        };
        self.verify_signature(secret, signable_body)
    }

    pub(crate) fn verify_payload_sha256(&self, actual: &str) -> Result<(), SigV4Error> {
        match &self.payload_mode {
            PayloadMode::Precomputed(expected) if !constant_time_eq(actual, expected) => {
                Err(SigV4Error::PayloadHashMismatch)
            }
            PayloadMode::Calculate => Err(SigV4Error::StreamingPayloadHashRequired),
            PayloadMode::Unsigned | PayloadMode::Precomputed(_) => Ok(()),
        }
    }

    fn verify_signature(
        &self,
        secret: &str,
        signable_body: SignableBody<'_>,
    ) -> Result<(), SigV4Error> {
        let identity = Credentials::new(
            self.access_key_id.clone(),
            secret.to_owned(),
            None,
            None,
            "mediahub-application-access-key",
        )
        .into();
        let mut settings = SigningSettings::default();
        settings.signature_location = match self.location {
            AuthLocation::Header => SignatureLocation::Headers,
            AuthLocation::Query => SignatureLocation::QueryParams,
        };
        settings.expires_in = self.expires_in;
        settings.percent_encoding_mode = PercentEncodingMode::Single;
        settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
        settings.payload_checksum_kind = PayloadChecksumKind::NoHeader;
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name(SERVICE)
            .time(self.signing_time)
            .settings(settings)
            .build()
            .map_err(|_| SigV4Error::InvalidRequest)?
            .into();
        let signable = SignableRequest::new(
            &self.method,
            self.signing_uri.as_str(),
            self.signed_headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
            signable_body,
        )
        .map_err(|_| SigV4Error::InvalidRequest)?;
        let expected = sign(signable, &params).map_err(|_| SigV4Error::InvalidRequest)?;
        if constant_time_eq(expected.signature(), &self.supplied_signature) {
            Ok(())
        } else {
            Err(SigV4Error::SignatureMismatch)
        }
    }

    fn parse_header(
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
        authorization: &str,
    ) -> Result<Self, SigV4Error> {
        let fields = authorization
            .strip_prefix(ALGORITHM)
            .ok_or(SigV4Error::UnsupportedAlgorithm)?
            .trim()
            .split(',')
            .map(str::trim)
            .map(|field| field.split_once('=').ok_or(SigV4Error::InvalidRequest))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let credential = fields.get("Credential").ok_or(SigV4Error::InvalidRequest)?;
        let declared_headers = fields
            .get("SignedHeaders")
            .ok_or(SigV4Error::InvalidRequest)?;
        let supplied_signature = fields.get("Signature").ok_or(SigV4Error::InvalidRequest)?;
        validate_signature(supplied_signature)?;
        let scope = CredentialScope::parse(credential)?;
        let date = required_header(headers, "x-amz-date")?;
        let signing_time = parse_signing_time(date)?;
        scope.validate_date(signing_time)?;
        let signed_headers = collect_signed_headers(headers, uri, declared_headers)?;
        let payload_mode = payload_mode(headers, false)?;
        Ok(Self {
            access_key_id: scope.access_key_id,
            region: scope.region,
            signing_time: signing_time.into(),
            location: AuthLocation::Header,
            expires_in: None,
            method: method.as_str().to_owned(),
            signing_uri: uri.to_string(),
            signed_headers,
            payload_mode,
            supplied_signature: (*supplied_signature).to_owned(),
        })
    }

    fn parse_query(method: &Method, uri: &Uri, headers: &HeaderMap) -> Result<Self, SigV4Error> {
        let values = query_values(uri)?;
        if required_query(&values, "X-Amz-Algorithm")? != ALGORITHM {
            return Err(SigV4Error::UnsupportedAlgorithm);
        }
        if values.contains_key("X-Amz-Security-Token") {
            return Err(SigV4Error::SessionCredentialsUnsupported);
        }
        let scope = CredentialScope::parse(required_query(&values, "X-Amz-Credential")?)?;
        let signing_time = parse_signing_time(required_query(&values, "X-Amz-Date")?)?;
        scope.validate_date(signing_time)?;
        let expires_in = required_query(&values, "X-Amz-Expires")?
            .parse::<u64>()
            .ok()
            .map(Duration::from_secs)
            .filter(|value| !value.is_zero() && *value <= MAX_PRESIGN_EXPIRY)
            .ok_or(SigV4Error::InvalidExpiry)?;
        let declared_headers = required_query(&values, "X-Amz-SignedHeaders")?;
        let supplied_signature = required_query(&values, "X-Amz-Signature")?;
        validate_signature(supplied_signature)?;
        let signed_headers = collect_signed_headers(headers, uri, declared_headers)?;
        Ok(Self {
            access_key_id: scope.access_key_id,
            region: scope.region,
            signing_time: signing_time.into(),
            location: AuthLocation::Query,
            expires_in: Some(expires_in),
            method: method.as_str().to_owned(),
            signing_uri: unsigned_query_uri(uri)?,
            signed_headers,
            payload_mode: PayloadMode::Unsigned,
            supplied_signature: supplied_signature.to_owned(),
        })
    }

    fn validate_time(&self, now: SystemTime) -> Result<(), SigV4Error> {
        if self.signing_time > now {
            if self
                .signing_time
                .duration_since(now)
                .map_err(|_| SigV4Error::Expired)?
                > MAX_HEADER_SKEW
            {
                return Err(SigV4Error::Expired);
            }
            return Ok(());
        }
        let age = now
            .duration_since(self.signing_time)
            .map_err(|_| SigV4Error::Expired)?;
        match self.location {
            AuthLocation::Header if age > MAX_HEADER_SKEW => Err(SigV4Error::Expired),
            AuthLocation::Query if age > self.expires_in.ok_or(SigV4Error::InvalidExpiry)? => {
                Err(SigV4Error::Expired)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug)]
struct CredentialScope {
    access_key_id: String,
    date: String,
    region: String,
}

impl CredentialScope {
    fn parse(value: &str) -> Result<Self, SigV4Error> {
        let mut fields = value.split('/');
        let (Some(access_key_id), Some(date), Some(region), Some(service), Some(terminator), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(SigV4Error::InvalidCredentialScope);
        };
        if access_key_id.is_empty()
            || access_key_id.len() > 256
            || date.len() != 8
            || !date.bytes().all(|byte| byte.is_ascii_digit())
            || region.is_empty()
            || region.len() > 128
            || service != SERVICE
            || terminator != TERMINATOR
        {
            return Err(SigV4Error::InvalidCredentialScope);
        }
        Ok(Self {
            access_key_id: access_key_id.to_owned(),
            date: date.to_owned(),
            region: region.to_owned(),
        })
    }

    fn validate_date(&self, signing_time: DateTime<Utc>) -> Result<(), SigV4Error> {
        if signing_time.format("%Y%m%d").to_string() == self.date {
            Ok(())
        } else {
            Err(SigV4Error::InvalidCredentialScope)
        }
    }
}

fn parse_signing_time(value: &str) -> Result<DateTime<Utc>, SigV4Error> {
    NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ")
        .map(|value| value.and_utc())
        .map_err(|_| SigV4Error::InvalidDate)
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, SigV4Error> {
    headers
        .get(name)
        .ok_or(SigV4Error::MissingAuthentication)?
        .to_str()
        .map_err(|_| SigV4Error::InvalidRequest)
}

fn collect_signed_headers(
    headers: &HeaderMap,
    uri: &Uri,
    declared: &str,
) -> Result<Vec<(String, String)>, SigV4Error> {
    let names = declared.split(';').collect::<Vec<_>>();
    if names.is_empty()
        || !names.contains(&"host")
        || names.windows(2).any(|pair| pair[0] >= pair[1])
        || names.iter().any(|name| {
            name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
    {
        return Err(SigV4Error::InvalidSignedHeaders);
    }
    let mut result = Vec::new();
    for name in names {
        let mut found = false;
        for value in headers.get_all(name).iter() {
            result.push((
                name.to_owned(),
                value
                    .to_str()
                    .map_err(|_| SigV4Error::InvalidSignedHeaders)?
                    .to_owned(),
            ));
            found = true;
        }
        if !found
            && name == "host"
            && let Some(authority) = uri.authority()
        {
            result.push((name.to_owned(), authority.as_str().to_owned()));
            found = true;
        }
        if !found {
            return Err(SigV4Error::InvalidSignedHeaders);
        }
    }
    Ok(result)
}

fn payload_mode(headers: &HeaderMap, presigned: bool) -> Result<PayloadMode, SigV4Error> {
    if presigned {
        return Ok(PayloadMode::Unsigned);
    }
    let Some(value) = headers
        .get("x-amz-content-sha256")
        .map(|value| value.to_str().map_err(|_| SigV4Error::InvalidPayloadHash))
        .transpose()?
    else {
        return Ok(PayloadMode::Calculate);
    };
    if value == "UNSIGNED-PAYLOAD" {
        return Ok(PayloadMode::Unsigned);
    }
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(PayloadMode::Precomputed(value.to_ascii_lowercase()));
    }
    Err(SigV4Error::InvalidPayloadHash)
}

fn query_values(uri: &Uri) -> Result<BTreeMap<String, String>, SigV4Error> {
    let mut values = BTreeMap::new();
    for (name, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        if values
            .insert(name.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(SigV4Error::DuplicateQueryParameter);
        }
    }
    Ok(values)
}

fn required_query<'a>(
    values: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str, SigV4Error> {
    values
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(SigV4Error::MissingAuthentication)
}

fn unsigned_query_uri(uri: &Uri) -> Result<String, SigV4Error> {
    const AUTH_PARAMETERS: [&str; 6] = [
        "X-Amz-Algorithm",
        "X-Amz-Credential",
        "X-Amz-Date",
        "X-Amz-Expires",
        "X-Amz-SignedHeaders",
        "X-Amz-Signature",
    ];
    let retained = uri
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            let raw_name = pair.split_once('=').map_or(*pair, |(name, _)| name);
            let decoded = url::form_urlencoded::parse(format!("{raw_name}=").as_bytes())
                .next()
                .map(|(name, _)| name.into_owned())
                .unwrap_or_default();
            !AUTH_PARAMETERS.contains(&decoded.as_str())
        })
        .collect::<Vec<_>>()
        .join("&");
    let path = uri
        .path_and_query()
        .map_or(uri.path(), |value| value.path());
    if retained.is_empty() {
        Ok(path.to_owned())
    } else {
        Ok(format!("{path}?{retained}"))
    }
}

fn validate_signature(signature: &str) -> Result<(), SigV4Error> {
    if signature.len() == 64 && signature.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(SigV4Error::InvalidSignature)
    }
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    bool::from(left.as_bytes().ct_eq(right.as_bytes()))
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum SigV4Error {
    #[error("AWS Signature Version 4 authentication is required")]
    MissingAuthentication,
    #[error("the signing algorithm is not supported")]
    UnsupportedAlgorithm,
    #[error("the credential scope is invalid")]
    InvalidCredentialScope,
    #[error("the request date is invalid")]
    InvalidDate,
    #[error("the signed header list is invalid")]
    InvalidSignedHeaders,
    #[error("the signature is invalid")]
    InvalidSignature,
    #[error("the request signature does not match")]
    SignatureMismatch,
    #[error("the request signature has expired")]
    Expired,
    #[error("the presigned expiry is invalid")]
    InvalidExpiry,
    #[error("temporary session credentials are not supported")]
    SessionCredentialsUnsupported,
    #[error("a signing query parameter is duplicated")]
    DuplicateQueryParameter,
    #[error("the payload hash is invalid")]
    InvalidPayloadHash,
    #[error("the payload does not match x-amz-content-sha256")]
    PayloadHashMismatch,
    #[error("streaming uploads require x-amz-content-sha256 or UNSIGNED-PAYLOAD")]
    StreamingPayloadHashRequired,
    #[error("the signed request is invalid")]
    InvalidRequest,
}

impl SigV4Error {
    pub(crate) const fn s3_code(self) -> &'static str {
        match self {
            Self::MissingAuthentication => "AccessDenied",
            Self::UnsupportedAlgorithm
            | Self::SessionCredentialsUnsupported
            | Self::StreamingPayloadHashRequired => "InvalidRequest",
            Self::InvalidCredentialScope => "AuthorizationHeaderMalformed",
            Self::InvalidDate | Self::Expired => "RequestTimeTooSkewed",
            Self::InvalidExpiry => "AuthorizationQueryParametersError",
            Self::InvalidPayloadHash | Self::PayloadHashMismatch => "XAmzContentSHA256Mismatch",
            Self::InvalidSignedHeaders
            | Self::InvalidSignature
            | Self::SignatureMismatch
            | Self::DuplicateQueryParameter
            | Self::InvalidRequest => "SignatureDoesNotMatch",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use aws_credential_types::Credentials;
    use aws_sigv4::{
        http_request::{
            PayloadChecksumKind, PercentEncodingMode, SignableBody, SignableRequest,
            SignatureLocation, SigningSettings, UriPathNormalizationMode, sign,
        },
        sign::v4,
    };
    use http::{Method, Request};
    use sha2::Digest;

    use super::{ParsedSigV4, SigV4Error};

    const ACCESS_KEY: &str = "mh_ak_sub2api";
    const SECRET: &str = "mediahub-sub2api-secret";

    fn signing_time() -> SystemTime {
        "2026-07-19T00:00:00Z"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .expect("timestamp")
            .into()
    }

    fn settings(location: SignatureLocation) -> SigningSettings {
        let mut settings = SigningSettings::default();
        settings.signature_location = location;
        settings.percent_encoding_mode = PercentEncodingMode::Single;
        settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
        settings.payload_checksum_kind = PayloadChecksumKind::NoHeader;
        settings
    }

    fn params(settings: SigningSettings) -> aws_sigv4::http_request::SigningParams<'static> {
        let identity = Box::leak(Box::new(
            Credentials::new(ACCESS_KEY, SECRET, None, None, "test").into(),
        ));
        v4::SigningParams::builder()
            .identity(identity)
            .region("us-east-1")
            .name("s3")
            .time(signing_time())
            .settings(settings)
            .build()
            .expect("signing params")
            .into()
    }

    #[test]
    fn verifies_header_signed_unsigned_put() {
        let body = b"generated-image".to_vec();
        let mut request = Request::builder()
            .method(Method::PUT)
            .uri("/s3/images/images/task-0.png")
            .header("host", "media.example.com")
            .header("content-type", "image/png")
            .header("x-amz-content-sha256", "UNSIGNED-PAYLOAD")
            .body(body.clone())
            .expect("request");
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str(), value.to_str().expect("header")));
        let signable = SignableRequest::new(
            "PUT",
            request.uri().to_string(),
            headers,
            SignableBody::UnsignedPayload,
        )
        .expect("signable");
        sign(signable, &params(settings(SignatureLocation::Headers)))
            .expect("signature")
            .into_parts()
            .0
            .apply_to_request_http1x(&mut request);
        let parsed = ParsedSigV4::parse(
            request.method(),
            request.uri(),
            request.headers(),
            signing_time(),
        )
        .expect("parse");
        assert_eq!(parsed.access_key_id(), ACCESS_KEY);
        assert_eq!(parsed.verify(SECRET, &body), Ok(()));
        assert_eq!(
            parsed.verify("wrong-secret", &body),
            Err(SigV4Error::SignatureMismatch)
        );
    }

    #[test]
    fn streaming_upload_requires_an_explicit_payload_mode() {
        let body = b"generated-image".to_vec();
        let mut request = Request::builder()
            .method(Method::PUT)
            .uri("/s3/images/images/task-0.png")
            .header("host", "media.example.com")
            .body(body.clone())
            .expect("request");
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str(), value.to_str().expect("header")));
        let signable = SignableRequest::new(
            "PUT",
            request.uri().to_string(),
            headers,
            SignableBody::Bytes(&body),
        )
        .expect("signable");
        sign(signable, &params(settings(SignatureLocation::Headers)))
            .expect("signature")
            .into_parts()
            .0
            .apply_to_request_http1x(&mut request);
        let parsed = ParsedSigV4::parse(
            request.method(),
            request.uri(),
            request.headers(),
            signing_time(),
        )
        .expect("parse");
        assert_eq!(parsed.verify(SECRET, &body), Ok(()));
        assert_eq!(
            parsed.verify_streaming_signature(SECRET),
            Err(SigV4Error::StreamingPayloadHashRequired)
        );
    }

    #[test]
    fn verifies_presigned_get_and_rejects_expiry() {
        let mut request = Request::builder()
            .method(Method::GET)
            .uri("/s3/images/images/task-0.png?x-id=GetObject")
            .header("host", "media.example.com")
            .body(())
            .expect("request");
        let mut presign_settings = settings(SignatureLocation::QueryParams);
        presign_settings.expires_in = Some(Duration::from_secs(24 * 60 * 60));
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str(), value.to_str().expect("header")));
        let signable = SignableRequest::new(
            "GET",
            request.uri().to_string(),
            headers,
            SignableBody::UnsignedPayload,
        )
        .expect("signable");
        sign(signable, &params(presign_settings))
            .expect("signature")
            .into_parts()
            .0
            .apply_to_request_http1x(&mut request);
        let parsed = ParsedSigV4::parse(
            request.method(),
            request.uri(),
            request.headers(),
            signing_time() + Duration::from_secs(23 * 60 * 60),
        )
        .expect("parse");
        assert_eq!(parsed.verify(SECRET, &[]), Ok(()));
        assert!(matches!(
            ParsedSigV4::parse(
                request.method(),
                request.uri(),
                request.headers(),
                signing_time() + Duration::from_secs(24 * 60 * 60 + 1),
            ),
            Err(SigV4Error::Expired)
        ));
    }

    #[test]
    fn precomputed_payload_hash_is_independently_checked() {
        let body = b"generated-image".to_vec();
        let digest = hex::encode(sha2::Sha256::digest(&body));
        let mut request = Request::builder()
            .method(Method::PUT)
            .uri("/s3/images/task.png")
            .header("host", "media.example.com")
            .header("x-amz-content-sha256", &digest)
            .body(body.clone())
            .expect("request");
        let headers = request
            .headers()
            .iter()
            .map(|(name, value)| (name.as_str(), value.to_str().expect("header")));
        let signable = SignableRequest::new(
            "PUT",
            request.uri().to_string(),
            headers,
            SignableBody::Precomputed(digest),
        )
        .expect("signable");
        sign(signable, &params(settings(SignatureLocation::Headers)))
            .expect("signature")
            .into_parts()
            .0
            .apply_to_request_http1x(&mut request);
        let parsed = ParsedSigV4::parse(
            request.method(),
            request.uri(),
            request.headers(),
            signing_time(),
        )
        .expect("parse");
        assert_eq!(
            parsed.verify(SECRET, b"tampered"),
            Err(SigV4Error::PayloadHashMismatch)
        );
        assert_eq!(parsed.verify_streaming_signature(SECRET), Ok(()));
        assert_eq!(
            parsed.verify_payload_sha256(&hex::encode(sha2::Sha256::digest(&body))),
            Ok(())
        );
        assert_eq!(
            parsed.verify_payload_sha256(&hex::encode(sha2::Sha256::digest(b"tampered"))),
            Err(SigV4Error::PayloadHashMismatch)
        );
    }
}
