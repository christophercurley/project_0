use crate::error::{ApiError, Result};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub fn username(value: &str) -> Result<String> {
    if !(3..=32).contains(&value.len())
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        return Err(ApiError::bad());
    }
    Ok(value.to_ascii_lowercase())
}
pub fn password(value: &str) -> Result<()> {
    if !(12..=128).contains(&value.len()) {
        return Err(ApiError::bad());
    }
    Ok(())
}
pub fn hash(value: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| ApiError::busy())
}
pub fn verify(value: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| {
        Argon2::default()
            .verify_password(value.as_bytes(), &h)
            .is_ok()
    })
}
pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| ApiError::busy())?;
    Ok(bytes)
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn digest(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}
pub fn equal_secret(left: &str, right: &str) -> bool {
    bool::from(digest(left).ct_eq(&digest(right)))
}
pub fn token() -> Result<String> {
    Ok(hex(&random::<32>()?))
}
pub fn valid_token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
