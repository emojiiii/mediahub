use std::{env, net::SocketAddr};

use axum::http::HeaderValue;
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use mediahub_adapter_s3::S3Config;
use mediahub_server::identity::normalize_email;
use url::Url;

#[derive(Clone, Debug)]
pub(super) struct CookieConfig {
    pub(super) secure: bool,
    pub(super) same_site: &'static str,
}

#[derive(Clone)]
pub(super) struct ServerConfig {
    pub(super) bind_addr: SocketAddr,
    pub(super) database_url: String,
    pub(super) storage_root: String,
    pub(super) storage_backend: StorageBackend,
    pub(super) s3_config: Option<S3Config>,
    pub(super) access_key_master_key: String,
    pub(super) access_key_master_key_version: u32,
    pub(super) access_key_master_keyring: Vec<(u32, String)>,
    pub(super) media_url_signing_key: Vec<u8>,
    pub(super) cookie_config: CookieConfig,
    pub(super) cors_allowed_origins: Vec<HeaderValue>,
    pub(super) registration_enabled: bool,
    pub(super) expose_auth_tokens: bool,
    pub(super) email_provider: Option<EmailProviderConfig>,
    pub(super) bootstrap_admin_email: Option<String>,
    pub(super) metrics_bearer_token: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StorageBackend {
    Local,
    S3,
}

#[derive(Clone)]
pub(super) struct EmailProviderConfig {
    pub(super) url: Url,
    pub(super) bearer_token: String,
    pub(super) from: String,
}

fn validate_database_url(database_url: &str) -> Result<(), String> {
    let scheme = database_url
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .filter(|scheme| !scheme.is_empty())
        .ok_or_else(|| {
            "MEDIAHUB_DATABASE_URL must include a postgres or postgresql scheme".to_owned()
        })?;
    match scheme {
        "postgres" | "postgresql" => Ok(()),
        _ => Err(format!(
            "MEDIAHUB_DATABASE_URL must use PostgreSQL, got unsupported scheme {scheme:?}"
        )),
    }
}

fn storage_backend(value: &str) -> Result<StorageBackend, String> {
    match value {
        "local" => Ok(StorageBackend::Local),
        "s3" => Ok(StorageBackend::S3),
        value => Err(format!(
            "MEDIAHUB_STORAGE_BACKEND must be local or s3, got {value:?}"
        )),
    }
}

fn parse_boolean_config(name: &str, value: Option<&str>, default: bool) -> Result<bool, String> {
    match value {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(format!("{name} must be true or false")),
    }
}

fn boolean_env(name: &str, default: bool) -> Result<bool, String> {
    match env::var(name) {
        Ok(value) => parse_boolean_config(name, Some(&value), default),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8")),
    }
}

fn decode_media_signing_key(value: &str) -> Result<Vec<u8>, String> {
    let key = URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| STANDARD.decode(value))
        .map_err(|_| "MEDIAHUB_MEDIA_SIGNING_KEY must be base64 encoded".to_owned())?;
    if key.len() < 32 {
        return Err("MEDIAHUB_MEDIA_SIGNING_KEY must decode to at least 32 bytes".to_owned());
    }
    Ok(key)
}

impl ServerConfig {
    pub(super) fn from_env() -> Result<Self, String> {
        let bind_addr = env::var("MEDIAHUB_BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3000".into())
            .parse()
            .map_err(|error| format!("MEDIAHUB_BIND_ADDR is invalid: {error}"))?;
        let database_url = env::var("MEDIAHUB_DATABASE_URL")
            .map_err(|_| "MEDIAHUB_DATABASE_URL is required".to_owned())?;
        validate_database_url(&database_url)?;
        let storage_backend = storage_backend(
            &env::var("MEDIAHUB_STORAGE_BACKEND").unwrap_or_else(|_| "local".into()),
        )?;
        let s3_config = if storage_backend == StorageBackend::S3 {
            let access_key_id = env::var("MEDIAHUB_S3_ACCESS_KEY_ID").ok();
            let secret_access_key = env::var("MEDIAHUB_S3_SECRET_ACCESS_KEY").ok();
            if access_key_id.is_some() != secret_access_key.is_some() {
                return Err(
                    "MEDIAHUB_S3_ACCESS_KEY_ID and MEDIAHUB_S3_SECRET_ACCESS_KEY must be configured together"
                        .into(),
                );
            }
            let endpoint = env::var("MEDIAHUB_S3_ENDPOINT")
                .ok()
                .filter(|value| !value.trim().is_empty());
            let allow_http = boolean_env("MEDIAHUB_S3_ALLOW_HTTP", false)?;
            if endpoint
                .as_deref()
                .is_some_and(|value| value.starts_with("http://"))
                && !allow_http
            {
                return Err(
                    "MEDIAHUB_S3_ENDPOINT must use HTTPS unless MEDIAHUB_S3_ALLOW_HTTP=true".into(),
                );
            }
            Some(S3Config {
                bucket: env::var("MEDIAHUB_S3_BUCKET")
                    .map_err(|_| "MEDIAHUB_S3_BUCKET is required for S3 storage".to_owned())?,
                region: env::var("MEDIAHUB_S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
                endpoint,
                access_key_id,
                secret_access_key,
                session_token: env::var("MEDIAHUB_S3_SESSION_TOKEN").ok(),
                allow_http,
                virtual_hosted_style: boolean_env("MEDIAHUB_S3_VIRTUAL_HOSTED_STYLE", false)?,
                prefix: env::var("MEDIAHUB_S3_PREFIX")
                    .ok()
                    .filter(|value| !value.trim().is_empty()),
            })
        } else {
            None
        };
        let access_key_master_key = env::var("MEDIAHUB_ACCESS_KEY_MASTER_KEY")
            .map_err(|_| "MEDIAHUB_ACCESS_KEY_MASTER_KEY is required".to_owned())?;
        let access_key_master_key_version = env::var("MEDIAHUB_ACCESS_KEY_MASTER_KEY_VERSION")
            .unwrap_or_else(|_| "1".into())
            .parse()
            .map_err(|_| "MEDIAHUB_ACCESS_KEY_MASTER_KEY_VERSION is invalid".to_owned())?;
        let allow_insecure_cookies =
            env::var("MEDIAHUB_ALLOW_INSECURE_COOKIES").ok().as_deref() == Some("true");
        let same_site = match env::var("MEDIAHUB_COOKIE_SAME_SITE")
            .unwrap_or_else(|_| "lax".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "lax" => "Lax",
            "strict" => "Strict",
            "none" if !allow_insecure_cookies => "None",
            "none" => {
                return Err("MEDIAHUB_COOKIE_SAME_SITE=none requires secure cookies".to_owned());
            }
            _ => return Err("MEDIAHUB_COOKIE_SAME_SITE must be lax, strict, or none".to_owned()),
        };
        let cors_allowed_origins = env::var("MEDIAHUB_CORS_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(HeaderValue::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "MEDIAHUB_CORS_ALLOWED_ORIGINS contains an invalid origin".to_owned())?;
        let access_key_master_keyring = env::var("MEDIAHUB_ACCESS_KEY_MASTER_KEYRING")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                let (version, key) = value.split_once(':').ok_or_else(|| {
                    "MEDIAHUB_ACCESS_KEY_MASTER_KEYRING must use version:base64-key entries"
                        .to_owned()
                })?;
                let version = version.parse().map_err(|_| {
                    "MEDIAHUB_ACCESS_KEY_MASTER_KEYRING contains an invalid version".to_owned()
                })?;
                Ok((version, key.to_owned()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let media_url_signing_key = env::var("MEDIAHUB_MEDIA_SIGNING_KEY")
            .map_err(|_| "MEDIAHUB_MEDIA_SIGNING_KEY is required".to_owned())
            .and_then(|value| decode_media_signing_key(&value))?;
        let expose_auth_tokens =
            env::var("MEDIAHUB_EXPOSE_AUTH_TOKENS").ok().as_deref() == Some("true");
        let registration_enabled = boolean_env("MEDIAHUB_REGISTRATION_ENABLED", true)?;
        let bootstrap_admin_email = env::var("MEDIAHUB_BOOTSTRAP_ADMIN_EMAIL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                normalize_email(&value)
                    .map_err(|_| "MEDIAHUB_BOOTSTRAP_ADMIN_EMAIL is invalid".to_owned())
            })
            .transpose()?;
        let metrics_bearer_token = env::var("MEDIAHUB_METRICS_BEARER_TOKEN")
            .ok()
            .filter(|value| !value.is_empty())
            .map(|value| {
                if value.len() < 32
                    || value.len() > 512
                    || value.bytes().any(|byte| byte.is_ascii_control())
                {
                    Err(
                        "MEDIAHUB_METRICS_BEARER_TOKEN must contain 32-512 printable bytes"
                            .to_owned(),
                    )
                } else {
                    Ok(value)
                }
            })
            .transpose()?;
        let email_provider = match env::var("MEDIAHUB_EMAIL_PROVIDER_URL").ok() {
            Some(value) if !value.trim().is_empty() => {
                let url = Url::parse(value.trim())
                    .map_err(|_| "MEDIAHUB_EMAIL_PROVIDER_URL is invalid".to_owned())?;
                let allow_insecure_provider = env::var("MEDIAHUB_ALLOW_INSECURE_EMAIL_PROVIDER")
                    .ok()
                    .as_deref()
                    == Some("true");
                if url.scheme() != "https" && !(allow_insecure_provider && url.scheme() == "http") {
                    return Err(
                        "MEDIAHUB_EMAIL_PROVIDER_URL must use HTTPS unless the explicit local-development override is enabled"
                            .to_owned(),
                    );
                }
                Some(EmailProviderConfig {
                    url,
                    bearer_token: env::var("MEDIAHUB_EMAIL_PROVIDER_TOKEN").map_err(|_| {
                        "MEDIAHUB_EMAIL_PROVIDER_TOKEN is required with the provider URL".to_owned()
                    })?,
                    from: env::var("MEDIAHUB_EMAIL_FROM").map_err(|_| {
                        "MEDIAHUB_EMAIL_FROM is required with the provider URL".to_owned()
                    })?,
                })
            }
            _ if expose_auth_tokens => None,
            _ => {
                return Err(
                    "MEDIAHUB_EMAIL_PROVIDER_URL is required unless MEDIAHUB_EXPOSE_AUTH_TOKENS=true for isolated development"
                        .to_owned(),
                );
            }
        };
        Ok(Self {
            bind_addr,
            database_url,
            storage_root: env::var("MEDIAHUB_STORAGE_ROOT")
                .unwrap_or_else(|_| "data/storage".into()),
            storage_backend,
            s3_config,
            access_key_master_key,
            access_key_master_key_version,
            access_key_master_keyring,
            media_url_signing_key,
            cookie_config: CookieConfig {
                secure: !allow_insecure_cookies,
                same_site,
            },
            cors_allowed_origins,
            registration_enabled,
            expose_auth_tokens,
            email_provider,
            bootstrap_admin_email,
            metrics_bearer_token,
        })
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    use super::{
        StorageBackend, decode_media_signing_key, parse_boolean_config, storage_backend,
        validate_database_url,
    };

    #[test]
    fn registration_config_defaults_enabled_and_rejects_invalid_values() {
        assert_eq!(
            parse_boolean_config("MEDIAHUB_REGISTRATION_ENABLED", None, true),
            Ok(true)
        );
        assert_eq!(
            parse_boolean_config("MEDIAHUB_REGISTRATION_ENABLED", Some("false"), true),
            Ok(false)
        );
        assert_eq!(
            parse_boolean_config("MEDIAHUB_REGISTRATION_ENABLED", Some("true"), false),
            Ok(true)
        );
        assert_eq!(
            parse_boolean_config("MEDIAHUB_REGISTRATION_ENABLED", Some("disabled"), true),
            Err("MEDIAHUB_REGISTRATION_ENABLED must be true or false".into())
        );
    }

    #[test]
    fn database_url_accepts_only_postgresql_schemes() {
        assert_eq!(
            validate_database_url("postgres://mediahub:secret@db/mediahub"),
            Ok(())
        );
        assert_eq!(
            validate_database_url("postgresql://mediahub:secret@db/mediahub"),
            Ok(())
        );
        assert_eq!(
            validate_database_url("unsupported://db/mediahub"),
            Err(
                "MEDIAHUB_DATABASE_URL must use PostgreSQL, got unsupported scheme \"unsupported\""
                    .into()
            )
        );
        assert_eq!(
            validate_database_url("missing-scheme"),
            Err("MEDIAHUB_DATABASE_URL must include a postgres or postgresql scheme".into())
        );
    }

    #[test]
    fn storage_backend_selector_accepts_supported_backends() {
        assert_eq!(storage_backend("local"), Ok(StorageBackend::Local));
        assert_eq!(storage_backend("s3"), Ok(StorageBackend::S3));
        assert_eq!(
            storage_backend("filesystem"),
            Err("MEDIAHUB_STORAGE_BACKEND must be local or s3, got \"filesystem\"".into())
        );
    }

    #[test]
    fn media_signing_key_requires_valid_base64_with_at_least_32_bytes() {
        let encoded_key = URL_SAFE_NO_PAD.encode([9; 32]);
        assert_eq!(decode_media_signing_key(&encoded_key), Ok(vec![9; 32]));
        assert!(decode_media_signing_key("not a base64 key").is_err());
        assert!(decode_media_signing_key(&URL_SAFE_NO_PAD.encode([9; 31])).is_err());
    }
}
