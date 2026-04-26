use crate::error::AppError;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use http::{HeaderMap, header::AUTHORIZATION};
use rand::{Rng, distributions::Alphanumeric, thread_rng};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct TenantContext {
    pub tenant_id: Uuid,
}

pub async fn authenticate(headers: &HeaderMap, db: &PgPool) -> Result<TenantContext, AppError> {
    let header_value = headers
        .get(AUTHORIZATION)
        .ok_or(AppError::InvalidApiKey)?
        .to_str()
        .map_err(|_| AppError::InvalidApiKey)?;

    let api_key = header_value
        .strip_prefix("Bearer ")
        .ok_or(AppError::InvalidApiKey)?;
    let key_prefix = extract_key_prefix(api_key).ok_or(AppError::InvalidApiKey)?;

    let key_record = crate::db::find_active_api_key_by_prefix(db, key_prefix)
        .await?
        .ok_or(AppError::InvalidApiKey)?;

    verify_api_key(api_key, &key_record.key_hash)?;
    Ok(TenantContext {
        tenant_id: key_record.tenant_id,
    })
}

pub fn generate_api_key() -> Result<(String, String, String), AppError> {
    let mut rng = thread_rng();
    let key_prefix: String = (&mut rng)
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect::<String>()
        .to_lowercase();
    let secret: String = (&mut rng)
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let api_key = format!("map_live_{key_prefix}_{secret}");
    let salt = SaltString::generate(&mut OsRng);
    let key_hash = Argon2::default()
        .hash_password(api_key.as_bytes(), &salt)
        .map_err(|_| AppError::Internal("failed to hash api key".to_owned()))?
        .to_string();

    Ok((api_key, key_prefix, key_hash))
}

fn verify_api_key(api_key: &str, key_hash: &str) -> Result<(), AppError> {
    let parsed_hash = PasswordHash::new(key_hash)
        .map_err(|_| AppError::Internal("stored api key hash is invalid".to_owned()))?;

    Argon2::default()
        .verify_password(api_key.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::InvalidApiKey)
}

fn extract_key_prefix(api_key: &str) -> Option<&str> {
    api_key
        .strip_prefix("map_live_")?
        .split_once('_')
        .map(|(prefix, _)| prefix)
}
