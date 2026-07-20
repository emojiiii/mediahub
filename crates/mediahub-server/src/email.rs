use std::time::Duration;

use mediahub_core::OffsetDateTime;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;
use url::Url;

use super::{ApiError, server_config::ResendConfig};

const RESEND_EMAILS_URL: &str = "https://api.resend.com/emails";
const RESEND_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthEmailKind {
    VerifyEmail,
    ResetPassword,
}

impl AuthEmailKind {
    fn name(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::ResetPassword => "reset_password",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify-email",
            Self::ResetPassword => "reset-password",
        }
    }

    fn subject(self) -> &'static str {
        match self {
            Self::VerifyEmail => "Verify your MediaHub email",
            Self::ResetPassword => "Reset your MediaHub password",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::VerifyEmail => "Verify email",
            Self::ResetPassword => "Reset password",
        }
    }

    fn introduction(self) -> &'static str {
        match self {
            Self::VerifyEmail => "Use the link below to verify your MediaHub email address.",
            Self::ResetPassword => "Use the link below to reset your MediaHub password.",
        }
    }
}

pub(super) struct ResendEmailProvider {
    client: reqwest::Client,
    config: ResendConfig,
    endpoint: Url,
}

#[derive(Serialize)]
struct ResendEmailRequest<'a> {
    from: &'a str,
    to: [&'a str; 1],
    subject: &'static str,
    html: String,
    text: String,
}

#[derive(Deserialize)]
struct ResendEmailResponse {
    id: String,
}

struct RenderedEmail {
    html: String,
    text: String,
}

impl ResendEmailProvider {
    pub(super) fn new(config: ResendConfig) -> Self {
        Self::with_endpoint(
            config,
            Url::parse(RESEND_EMAILS_URL).expect("Resend email endpoint must be valid"),
        )
    }

    fn with_endpoint(config: ResendConfig, endpoint: Url) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
            endpoint,
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_endpoint(config: ResendConfig, endpoint: Url) -> Self {
        Self::with_endpoint(config, endpoint)
    }

    pub(super) async fn send_token(
        &self,
        to: &str,
        kind: AuthEmailKind,
        token: &str,
        expires_at: OffsetDateTime,
    ) -> Result<(), ApiError> {
        let rendered = self.render(kind, token, expires_at)?;
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.config.api_key)
            .header("Idempotency-Key", idempotency_key(kind, token))
            .timeout(RESEND_REQUEST_TIMEOUT)
            .json(&ResendEmailRequest {
                from: &self.config.from,
                to: [to],
                subject: kind.subject(),
                html: rendered.html,
                text: rendered.text,
            })
            .send()
            .await
            .map_err(|error| {
                warn!(error = %error, "Resend request failed");
                ApiError::unavailable("email delivery is unavailable")
            })?;
        if !response.status().is_success() {
            warn!(status = %response.status(), "Resend rejected the email");
            return Err(ApiError::unavailable("email delivery was rejected"));
        }
        let accepted = response
            .json::<ResendEmailResponse>()
            .await
            .map_err(|error| {
                warn!(error = %error, "Resend returned an invalid success response");
                ApiError::unavailable("email delivery returned an invalid response")
            })?;
        if accepted.id.trim().is_empty() {
            return Err(ApiError::unavailable(
                "email delivery returned an invalid response",
            ));
        }
        Ok(())
    }

    fn render(
        &self,
        kind: AuthEmailKind,
        token: &str,
        expires_at: OffsetDateTime,
    ) -> Result<RenderedEmail, ApiError> {
        let mut action_url = self.config.web_url.join(kind.path()).map_err(|error| {
            warn!(error = %error, "email action URL construction failed");
            ApiError::unavailable("email delivery configuration is invalid")
        })?;
        action_url.query_pairs_mut().append_pair("token", token);
        let introduction = kind.introduction();
        let action = kind.action();
        let expiration = expires_at.to_string();
        Ok(RenderedEmail {
            html: format!(
                "<!doctype html><html><body><h1>{action}</h1><p>{introduction}</p><p><a href=\"{action_url}\">{action}</a></p><p>This link expires at {expiration}.</p><p>If you did not request this, you can ignore this email.</p></body></html>"
            ),
            text: format!(
                "{introduction}\n\n{action}: {action_url}\n\nThis link expires at {expiration}.\n\nIf you did not request this, you can ignore this email."
            ),
        })
    }
}

fn idempotency_key(kind: AuthEmailKind, token: &str) -> String {
    format!(
        "mediahub-{}-{}",
        kind.name(),
        hex::encode(Sha256::digest(token.as_bytes()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ResendConfig {
        ResendConfig {
            api_key: "re_test".into(),
            from: "MediaHub <noreply@example.com>".into(),
            web_url: Url::parse("https://console.example.com").expect("Web URL"),
        }
    }

    #[test]
    fn templates_use_the_matching_console_route() {
        let provider = ResendEmailProvider::new(config());
        let expires_at = OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(30);
        let verify = provider
            .render(AuthEmailKind::VerifyEmail, "verify-token", expires_at)
            .expect("verification email");
        assert!(
            verify
                .html
                .contains("https://console.example.com/verify-email?token=verify-token")
        );
        assert!(verify.text.contains("Verify email"));

        let reset = provider
            .render(AuthEmailKind::ResetPassword, "reset-token", expires_at)
            .expect("password-reset email");
        assert!(
            reset
                .html
                .contains("https://console.example.com/reset-password?token=reset-token")
        );
        assert!(reset.text.contains("Reset password"));
    }

    #[test]
    fn idempotency_key_hashes_the_raw_token() {
        let key = idempotency_key(AuthEmailKind::VerifyEmail, "raw-token");
        assert!(key.starts_with("mediahub-verify_email-"));
        assert!(!key.contains("raw-token"));
        assert!(key.len() <= 256);
    }
}
