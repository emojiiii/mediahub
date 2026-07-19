use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("email address is invalid")]
    InvalidEmail,
    #[error("password must be at least 12 bytes")]
    PasswordTooShort,
    #[error("password hashing failed")]
    PasswordHashing,
}

pub fn normalize_email(value: &str) -> Result<String, IdentityError> {
    let normalized = value.trim().to_ascii_lowercase();
    let Some((local, domain)) = normalized.split_once('@') else {
        return Err(IdentityError::InvalidEmail);
    };
    if local.is_empty()
        || domain.is_empty()
        || normalized.len() > 320
        || normalized.bytes().any(|byte| byte.is_ascii_control())
        || domain.starts_with('.')
        || domain.ends_with('.')
        || !domain.contains('.')
    {
        return Err(IdentityError::InvalidEmail);
    }
    Ok(normalized)
}

pub fn hash_password(password: &str) -> Result<String, IdentityError> {
    if password.len() < 12 {
        return Err(IdentityError::PasswordTooShort);
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| IdentityError::PasswordHashing)
}

pub fn verify_password(encoded_hash: &str, password: &str) -> bool {
    PasswordHash::new(encoded_hash).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalization_is_stable_and_rejects_invalid_values() {
        assert_eq!(
            normalize_email("  OWNER@Example.COM "),
            Ok("owner@example.com".into())
        );
        assert_eq!(
            normalize_email("owner@example"),
            Err(IdentityError::InvalidEmail)
        );
    }

    #[test]
    fn argon2id_password_hashes_are_verifiable() {
        let password = "correct-horse-battery-staple";
        let hash = hash_password(password).expect("hash password");
        assert!(hash.starts_with("$argon2id$"));
        assert!(verify_password(&hash, password));
        assert!(!verify_password(&hash, "incorrect-password"));
        assert_eq!(
            hash_password("too-short"),
            Err(IdentityError::PasswordTooShort)
        );
    }
}
