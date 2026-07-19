use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

pub const MAX_SIGNATURE_AGE: Duration = Duration::minutes(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalRequest {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, Vec<String>>,
    pub headers: BTreeMap<String, String>,
    pub body_sha256: String,
    pub timestamp: DateTime<Utc>,
    pub nonce: String,
    pub idempotency_key: Option<String>,
}

impl CanonicalRequest {
    #[must_use]
    pub fn render(&self) -> String {
        let mut query_encoder = url::form_urlencoded::Serializer::new(String::new());
        for (key, values) in &self.query {
            let mut values = values.iter().collect::<Vec<_>>();
            values.sort();
            for value in values {
                query_encoder.append_pair(key, value);
            }
        }
        let query = query_encoder.finish();
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| format!("{}:{}", name.to_ascii_lowercase(), value.trim()))
            .collect::<Vec<_>>()
            .join("\n");

        [
            self.method.to_ascii_uppercase(),
            self.path.clone(),
            query,
            headers,
            self.body_sha256.clone(),
            self.timestamp
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            self.nonce.clone(),
            self.idempotency_key.clone().unwrap_or_default(),
        ]
        .join("\n")
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HmacError {
    #[error("request timestamp is outside the allowed window")]
    Expired,
    #[error("signature does not match")]
    InvalidSignature,
}

pub fn sign_hmac(secret: &[u8], request: &CanonicalRequest) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any size");
    mac.update(request.render().as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_hmac(
    secret: &[u8],
    supplied_signature: &str,
    request: &CanonicalRequest,
    now: DateTime<Utc>,
) -> Result<(), HmacError> {
    if (now - request.timestamp).abs() > MAX_SIGNATURE_AGE {
        return Err(HmacError::Expired);
    }

    let expected = sign_hmac(secret, request);
    let valid = expected
        .as_bytes()
        .ct_eq(supplied_signature.as_bytes())
        .into();
    if valid {
        Ok(())
    } else {
        Err(HmacError::InvalidSignature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> CanonicalRequest {
        CanonicalRequest {
            method: "post".into(),
            path: "/api/v1/media".into(),
            query: BTreeMap::from([
                ("tag".into(), vec!["b".into(), "a".into()]),
                ("cursor".into(), vec!["first".into()]),
            ]),
            headers: BTreeMap::from([("Content-Type".into(), " application/json ".into())]),
            body_sha256: "abc123".into(),
            timestamp: "2026-07-14T12:00:00Z".parse().expect("valid timestamp"),
            nonce: "nonce-123".into(),
            idempotency_key: Some("request-1".into()),
        }
    }

    #[test]
    fn canonical_request_sorts_query_values_and_normalizes_fields() {
        assert_eq!(
            request().render(),
            "POST\n/api/v1/media\ncursor=first&tag=a&tag=b\ncontent-type:application/json\nabc123\n2026-07-14T12:00:00Z\nnonce-123\nrequest-1"
        );
    }

    #[test]
    fn canonical_request_percent_encodes_query_pairs_after_sorting() {
        let mut request = request();
        request.query = BTreeMap::from([
            ("space key".into(), vec!["hello world".into()]),
            ("tag".into(), vec!["a&b".into(), "a b".into()]),
        ]);
        assert!(
            request
                .render()
                .contains("space+key=hello+world&tag=a+b&tag=a%26b")
        );
    }

    #[test]
    fn valid_signature_within_five_minutes_is_accepted() {
        let request = request();
        let signature = sign_hmac(b"private", &request);
        assert_eq!(
            verify_hmac(
                b"private",
                &signature,
                &request,
                "2026-07-14T12:04:59Z".parse().expect("valid timestamp"),
            ),
            Ok(())
        );
    }

    #[test]
    fn expired_or_tampered_signatures_are_rejected() {
        let request = request();
        assert_eq!(
            verify_hmac(
                b"private",
                "00",
                &request,
                "2026-07-14T12:01:00Z".parse().expect("valid timestamp"),
            ),
            Err(HmacError::InvalidSignature)
        );
        let signature = sign_hmac(b"private", &request);
        assert_eq!(
            verify_hmac(
                b"private",
                &signature,
                &request,
                "2026-07-14T12:05:01Z".parse().expect("valid timestamp"),
            ),
            Err(HmacError::Expired)
        );
    }
}
