// API error mapping and response encoding.

#[derive(Debug)]

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}
impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: message.into(),
        }
    }
    fn invalid_query(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "invalid_request",
            message: message.into(),
        }
    }
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "authentication is required".into(),
        }
    }
    fn unauthorized_with_message(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: message.into(),
        }
    }
    fn replay_detected() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "replay_detected",
            message: "HMAC nonce has already been used".into(),
        }
    }
    fn idempotency_in_progress() -> Self {
        Self {
            status: StatusCode::ACCEPTED,
            code: "idempotency_in_progress",
            message: "an identical request is still in progress".into(),
        }
    }
    fn idempotency_conflict() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "idempotency_conflict",
            message: "Idempotency-Key was previously used with a different request".into(),
        }
    }
    fn invalid_credentials() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "invalid_credentials",
            message: "email or password is invalid".into(),
        }
    }
    fn invalid_one_time_token() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_token",
            message: "token is invalid, expired, or already used".into(),
        }
    }
    fn rate_limited() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate_limited",
            message: "too many authentication attempts; try again later".into(),
        }
    }
    fn registration_disabled() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "registration_disabled",
            message: "public registration is disabled".into(),
        }
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }
    fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            message: message.into(),
        }
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict",
            message: message.into(),
        }
    }
    fn payload_too_large(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "payload_too_large",
            message: message.into(),
        }
    }
    fn unsupported_media_type(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            code: "unsupported_media_type",
            message: message.into(),
        }
    }
    fn unprocessable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "unprocessable_entity",
            message: message.into(),
        }
    }
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "unavailable",
            message: message.into(),
        }
    }
    fn range_not_satisfiable() -> Self {
        Self {
            status: StatusCode::RANGE_NOT_SATISFIABLE,
            code: "invalid_range",
            message: "range is not satisfiable".into(),
        }
    }
    fn from_identity(error: mediahub_server::identity::IdentityError) -> Self {
        Self::bad_request(error.to_string())
    }
    fn from_access_key_cipher(error: AccessKeyCipherError) -> Self {
        warn!(error = %error, "access key cryptography failed");
        Self::unavailable("access key verification is unavailable")
    }
    fn from_hmac(error: HmacError) -> Self {
        match error {
            HmacError::Expired => Self::unauthorized_with_message("HMAC signature has expired"),
            HmacError::InvalidSignature => {
                Self::unauthorized_with_message("HMAC signature is invalid")
            }
        }
    }
    fn from_repository(error: mediahub_app::RepositoryError) -> Self {
        match error {
            mediahub_app::RepositoryError::NotFound => Self::not_found("resource not found"),
            mediahub_app::RepositoryError::Conflict => Self {
                status: StatusCode::CONFLICT,
                code: "conflict",
                message: "resource already exists or changed".into(),
            },
            mediahub_app::RepositoryError::QuotaExceeded => Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                code: "quota_exceeded",
                message: "application quota is exhausted".into(),
            },
            mediahub_app::RepositoryError::Invariant(_)
            | mediahub_app::RepositoryError::Unavailable(_) => {
                warn!(error = %error, "repository request failed");
                Self::unavailable("storage metadata is unavailable")
            }
        }
    }
    fn from_application(error: ApplicationError) -> Self {
        match error {
            ApplicationError::BucketNotFound => Self::not_found("bucket not found"),
            ApplicationError::BucketDoesNotBelongToApplication => {
                Self::not_found("bucket not found")
            }
            ApplicationError::ObjectAlreadyExists => Self {
                status: StatusCode::CONFLICT,
                code: "object_exists",
                message: "object key already exists".into(),
            },
            ApplicationError::QuotaExceeded => Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                code: "quota_exceeded",
                message: "application quota is exhausted".into(),
            },
            ApplicationError::UploadSessionNotFound
            | ApplicationError::UploadSessionDoesNotBelongToApplication => {
                Self::not_found("upload session not found")
            }
            ApplicationError::UploadSessionExpired
            | ApplicationError::UploadSessionCancelled
            | ApplicationError::UploadSessionAlreadyCompleted => Self::conflict(error.to_string()),
            ApplicationError::UploadSessionVerificationFailed => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: "upload_session_verification_failed",
                message: error.to_string(),
            },
            ApplicationError::Domain(error @ DomainError::ObjectTooLarge { .. }) => {
                Self::payload_too_large(error.to_string())
            }
            ApplicationError::Domain(error @ DomainError::MimeTypeNotAllowed { .. }) => {
                Self::unsupported_media_type(error.to_string())
            }
            ApplicationError::Domain(error) => Self::bad_request(error.to_string()),
            ApplicationError::Repository(error) => Self::from_repository(error),
            ApplicationError::ObjectStore(_) => Self::unavailable("object storage is unavailable"),
        }
    }
    fn from_variant(error: VariantApplicationError) -> Self {
        match error {
            VariantApplicationError::GenerationInProgress => Self {
                status: StatusCode::ACCEPTED,
                code: "variant_in_progress",
                message: error.to_string(),
            },
            VariantApplicationError::InvalidTransform => Self::bad_request(error.to_string()),
            VariantApplicationError::OutputTooLarge
            | VariantApplicationError::Processor(
                mediahub_app::ImageProcessorError::InputTooLarge,
            )
            | VariantApplicationError::Processor(
                mediahub_app::ImageProcessorError::OutputTooLarge,
            ) => Self::payload_too_large(error.to_string()),
            VariantApplicationError::Processor(
                mediahub_app::ImageProcessorError::UnsupportedInput,
            ) => Self::unsupported_media_type(error.to_string()),
            VariantApplicationError::Processor(_) => Self::unprocessable(error.to_string()),
            VariantApplicationError::LeaseLost => Self::unavailable(error.to_string()),
            VariantApplicationError::Storage(_) => {
                Self::unavailable("variant object storage is unavailable")
            }
            VariantApplicationError::Repository(error) => Self::from_repository(error),
        }
    }
    fn from_async_job(error: AsyncJobApplicationError) -> Self {
        match error {
            AsyncJobApplicationError::NotFound => Self::not_found("job not found"),
            AsyncJobApplicationError::DuplicateMediaIds
            | AsyncJobApplicationError::TooManyItems
            | AsyncJobApplicationError::InvalidClaimLimit
            | AsyncJobApplicationError::Domain(_) => Self::bad_request(error.to_string()),
            AsyncJobApplicationError::IdempotencyConflict => Self::idempotency_conflict(),
            AsyncJobApplicationError::AlreadyCompleted
            | AsyncJobApplicationError::AlreadyFailed => Self::conflict(error.to_string()),
            AsyncJobApplicationError::LeaseLost => Self::unavailable(error.to_string()),
            AsyncJobApplicationError::Repository(error) => Self::from_repository(error),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": { "code": self.code, "message": self.message } })),
        )
            .into_response()
    }
}

