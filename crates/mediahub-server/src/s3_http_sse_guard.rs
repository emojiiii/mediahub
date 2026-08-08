// Fail-closed protocol guard for S3 server-side encryption request headers.
//
// PrismArk must not accept or echo an SSE mode until encrypted persistence and
// authenticated reads are implemented end to end. Keep this guard independent
// from individual handlers so every S3 operation applies the same rule.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum S3SseOperationContext {
    PutObject,
    GetObject,
    HeadObject,
    CopyObject,
    CreateMultipartUpload,
    UploadPart,
    UploadPartCopy,
    CompleteMultipartUpload,
    PostObject,
    DeleteObject,
}

impl S3SseOperationContext {
    const fn description(self) -> &'static str {
        match self {
            Self::PutObject => "PutObject",
            Self::GetObject => "GetObject",
            Self::HeadObject => "HeadObject",
            Self::CopyObject => "CopyObject",
            Self::CreateMultipartUpload => "CreateMultipartUpload",
            Self::UploadPart => "UploadPart",
            Self::UploadPartCopy => "UploadPartCopy",
            Self::CompleteMultipartUpload => "CompleteMultipartUpload",
            Self::PostObject => "PostObject",
            Self::DeleteObject => "DeleteObject",
        }
    }
}

const S3_SSE_HEADER_PREFIX: &str = "x-amz-server-side-encryption";
const S3_COPY_SOURCE_SSE_HEADER_PREFIX: &str = "x-amz-copy-source-server-side-encryption";

pub(super) fn reject_unimplemented_s3_sse_headers(
    headers: &HeaderMap,
    operation: S3SseOperationContext,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    let contains_sse_header = headers.keys().any(|name| {
        let name = name.as_str();
        name == S3_SSE_HEADER_PREFIX
            || name.starts_with(concat!("x-amz-server-side-encryption", "-"))
            || name == S3_COPY_SOURCE_SSE_HEADER_PREFIX
            || name.starts_with(concat!("x-amz-copy-source-server-side-encryption", "-"))
    });

    if contains_sse_header {
        return Err(S3ApiError::not_implemented(
            format!(
                "Server-side encryption request headers are not supported for {}.",
                operation.description()
            ),
            resource,
            request_id,
        ));
    }

    Ok(())
}

#[cfg(test)]
mod s3_sse_guard_tests {
    use super::*;

    const RESOURCE: &str = "/bucket/key";
    const REQUEST_ID: &str = "sse-guard-test";

    fn assert_not_implemented(headers: &HeaderMap, operation: S3SseOperationContext) {
        let error = reject_unimplemented_s3_sse_headers(headers, operation, RESOURCE, REQUEST_ID)
            .expect_err("SSE headers must fail closed");

        assert_eq!(error.status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(error.code, "NotImplemented");
        assert_eq!(error.resource, RESOURCE);
        assert_eq!(error.request_id, REQUEST_ID);
        assert!(error.message.contains(operation.description()));
    }

    #[test]
    fn request_without_sse_headers_passes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/octet-stream"),
        );
        headers.insert(
            "x-amz-meta-encryption",
            HeaderValue::from_static("client-side"),
        );

        assert!(
            reject_unimplemented_s3_sse_headers(
                &headers,
                S3SseOperationContext::PutObject,
                RESOURCE,
                REQUEST_ID,
            )
            .is_ok()
        );
    }

    #[test]
    fn sse_s3_and_kms_headers_are_rejected() {
        for name in [
            "x-amz-server-side-encryption",
            "x-amz-server-side-encryption-aws-kms-key-id",
            "x-amz-server-side-encryption-context",
            "x-amz-server-side-encryption-bucket-key-enabled",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_static("test-value"));
            assert_not_implemented(&headers, S3SseOperationContext::PutObject);
        }
    }

    #[test]
    fn sse_customer_key_headers_are_rejected_without_reading_values() {
        for name in [
            "x-amz-server-side-encryption-customer-algorithm",
            "x-amz-server-side-encryption-customer-key",
            "x-amz-server-side-encryption-customer-key-md5",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_static("secret-test-value"));
            assert_not_implemented(&headers, S3SseOperationContext::PutObject);
        }
    }

    #[test]
    fn copy_source_customer_key_headers_are_rejected() {
        for name in [
            "x-amz-copy-source-server-side-encryption-customer-algorithm",
            "x-amz-copy-source-server-side-encryption-customer-key",
            "x-amz-copy-source-server-side-encryption-customer-key-md5",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_static("secret-test-value"));
            assert_not_implemented(&headers, S3SseOperationContext::CopyObject);
        }
    }

    #[test]
    fn header_names_are_case_insensitive_via_header_map() {
        let mut headers = HeaderMap::new();
        let mixed_case = HeaderName::from_bytes(b"X-AmZ-SeRvEr-SiDe-EnCrYpTiOn")
            .expect("mixed-case header name must be valid");
        headers.insert(mixed_case, HeaderValue::from_static("AES256"));

        assert_not_implemented(&headers, S3SseOperationContext::PutObject);
    }

    #[test]
    fn repeated_sse_headers_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.append(
            "x-amz-server-side-encryption",
            HeaderValue::from_static("AES256"),
        );
        headers.append(
            "x-amz-server-side-encryption",
            HeaderValue::from_static("aws:kms"),
        );

        assert_eq!(
            headers
                .get_all("x-amz-server-side-encryption")
                .iter()
                .count(),
            2
        );
        assert_not_implemented(&headers, S3SseOperationContext::PutObject);
    }

    #[test]
    fn get_and_head_request_side_sse_headers_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-server-side-encryption",
            HeaderValue::from_static("AES256"),
        );

        assert_not_implemented(&headers, S3SseOperationContext::GetObject);
        assert_not_implemented(&headers, S3SseOperationContext::HeadObject);
    }

    #[test]
    fn unknown_headers_in_sse_namespaces_fail_closed() {
        for name in [
            "x-amz-server-side-encryption-future-mode",
            "x-amz-copy-source-server-side-encryption-future-mode",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_static("future-value"));
            assert_not_implemented(&headers, S3SseOperationContext::UploadPartCopy);
        }
    }

    #[test]
    fn every_operation_context_uses_the_same_guard() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-server-side-encryption",
            HeaderValue::from_static("AES256"),
        );

        for operation in [
            S3SseOperationContext::PutObject,
            S3SseOperationContext::GetObject,
            S3SseOperationContext::HeadObject,
            S3SseOperationContext::CopyObject,
            S3SseOperationContext::CreateMultipartUpload,
            S3SseOperationContext::UploadPart,
            S3SseOperationContext::UploadPartCopy,
            S3SseOperationContext::CompleteMultipartUpload,
            S3SseOperationContext::PostObject,
            S3SseOperationContext::DeleteObject,
        ] {
            assert_not_implemented(&headers, operation);
        }
    }
}
