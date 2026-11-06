use axum::{Json, extract::State, http::StatusCode};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    config::Config,
    entity::{user, user_token},
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub iat: usize,
    pub exp: usize,
}

fn sign(config: &Config, sub: &str, role: &str, refresh: bool) -> AppResult<String> {
    let now = Utc::now().timestamp() as usize;
    let expires_in = if refresh { 7 * 24 * 60 * 60 } else { 15 * 60 };
    let claims = Claims {
        sub: sub.to_owned(),
        role: role.to_owned(),
        iat: now,
        exp: now + expires_in,
    };
    let secret = if refresh {
        &config.refresh_jwt_secret
    } else {
        &config.access_jwt_secret
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(e.to_string()))
}

fn verify(secret: &str, token: &str) -> AppResult<Claims> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|_| {
        AppError::Unauthorized("🚪 You are not logged in. Please sign in to continue! 🔐".into())
    })
}

pub fn verify_access(config: &Config, token: &str) -> AppResult<Claims> {
    verify(&config.access_jwt_secret, token)
}

pub fn verify_refresh(config: &Config, token: &str) -> AppResult<Claims> {
    verify(&config.refresh_jwt_secret, token)
}

pub fn create_access(config: &Config, sub: &str, role: &str) -> AppResult<String> {
    sign(config, sub, role, false)
}

// Matches the non-dummy SessionService.createToken path in NestJS, including
// the five-device limit and eviction of the earliest-expiring session.
// No public login route invokes this yet; it remains available to later features.
pub async fn create_tokens(
    state: &AppState,
    sub: &str,
    role: &str,
    previous: Option<&str>,
    browser_info: String,
) -> AppResult<(String, String)> {
    let user_id =
        uuid::Uuid::parse_str(sub).map_err(|_| AppError::BadRequest("invalid user ID".into()))?;
    let access = create_access(&state.config, sub, role)?;
    let refresh = sign(&state.config, sub, role, true)?;
    let now = Utc::now().naive_utc();
    let tx = state.db.begin().await.map_err(AppError::db)?;
    user::Entity::find_by_id(user_id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    if let Some(previous) = previous.filter(|token| !token.is_empty()) {
        let row = user_token::Entity::find()
            .filter(user_token::Column::Token.eq(previous))
            .filter(user_token::Column::UserId.eq(user_id))
            .one(&tx)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::Unauthorized("Refresh token not found".into()))?;
        if row.expired_at > now {
            return Err(AppError::Forbidden(
                "✅ You are already logged in. No need to log in again! 🎉".into(),
            ));
        }
        let mut active: user_token::ActiveModel = row.into();
        active.token = Set(refresh.clone());
        active.expired_at = Set(now + Duration::days(7));
        active.updated_at = Set(now);
        active.update(&tx).await.map_err(AppError::db)?;
    } else {
        let count = user_token::Entity::find()
            .filter(user_token::Column::UserId.eq(user_id))
            .count(&tx)
            .await
            .map_err(AppError::db)?;
        if count >= 5
            && let Some(oldest) = user_token::Entity::find()
                .filter(user_token::Column::UserId.eq(user_id))
                .order_by_asc(user_token::Column::ExpiredAt)
                .one(&tx)
                .await
                .map_err(AppError::db)?
        {
            user_token::Entity::delete_by_id(oldest.id)
                .exec(&tx)
                .await
                .map_err(AppError::db)?;
        }
        user_token::ActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            user_id: Set(user_id),
            token: Set(refresh.clone()),
            browser_info: Set(browser_info),
            expired_at: Set(now + Duration::days(7)),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&tx)
        .await
        .map_err(AppError::db)?;
    }
    tx.commit().await.map_err(AppError::db)?;
    Ok((access, refresh))
}

pub async fn create_dummy_tokens(
    state: &AppState,
    sub: &str,
    role: &str,
    previous: Option<&str>,
    browser_info: String,
) -> AppResult<(String, String)> {
    let access = create_access(&state.config, sub, role)?;
    let refresh = sign(&state.config, sub, role, true)?;
    let now = Utc::now().naive_utc();
    let expires = now + Duration::days(7);
    let user_id =
        uuid::Uuid::parse_str(sub).map_err(|_| AppError::Internal("invalid user ID".into()))?;

    if let Some(token) = previous.filter(|s| !s.is_empty()) {
        let old = user_token::Entity::find()
            .filter(user_token::Column::Token.eq(token))
            .filter(user_token::Column::UserId.eq(user_id))
            .one(&state.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::Unauthorized("Refresh token not found".into()))?;
        if old.expired_at > now {
            return Err(AppError::Forbidden(
                "✅ You are already logged in. No need to log in again! 🎉".into(),
            ));
        }
        let mut active: user_token::ActiveModel = old.into();
        active.token = Set(refresh.clone());
        active.expired_at = Set(expires);
        active.updated_at = Set(now);
        active.update(&state.db).await.map_err(AppError::db)?;
    } else {
        user_token::ActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            user_id: Set(user_id),
            token: Set(refresh.clone()),
            browser_info: Set(browser_info),
            expired_at: Set(expires),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&state.db)
        .await
        .map_err(AppError::db)?;
    }
    Ok((access, refresh))
}

fn cookie_token(jar: &CookieJar) -> AppResult<&str> {
    jar.get("refreshToken").map(|c| c.value()).ok_or_else(|| {
        AppError::Unauthorized("🚪 You are not logged in. Please sign in to continue! 🔐".into())
    })
}

pub async fn valid_stored_refresh(state: &AppState, token: &str) -> AppResult<Claims> {
    let claims = verify_refresh(&state.config, token)?;
    let stored = user_token::Entity::find()
        .filter(user_token::Column::Token.eq(token))
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::Unauthorized("Session no longer exists".into()))?;
    if stored.expired_at <= Utc::now().naive_utc() || stored.user_id.to_string() != claims.sub {
        return Err(AppError::Unauthorized("Session expired".into()));
    }
    Ok(claims)
}

pub async fn refresh(
    State(state): State<AppState>,
    jar: CookieJar,
) -> AppResult<(StatusCode, Json<Value>)> {
    let claims = valid_stored_refresh(&state, cookie_token(&jar)?).await?;
    // Legacy returns a refresh-signed JWT as accessToken. We intentionally
    // issue a real 15-minute access token so Bearer authentication works.
    let access = create_access(&state.config, &claims.sub, &claims.role)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {"accessToken": access}})),
    ))
}

pub async fn clear_token(state: &AppState, token: &str) -> AppResult<()> {
    valid_stored_refresh(state, token).await?;
    let result = user_token::Entity::delete_many()
        .filter(user_token::Column::Token.eq(token))
        .exec(&state.db)
        .await
        .map_err(AppError::db)?;
    if result.rows_affected == 0 {
        return Err(AppError::NotFound("Token not found".into()));
    }
    Ok(())
}

pub async fn clear_all(state: &AppState, token: &str) -> AppResult<()> {
    let claims = valid_stored_refresh(state, token).await?;
    let user_id = uuid::Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized("Invalid session".into()))?;
    let result = user_token::Entity::delete_many()
        .filter(user_token::Column::UserId.eq(user_id))
        .exec(&state.db)
        .await
        .map_err(AppError::db)?;
    if result.rows_affected == 0 {
        return Err(AppError::NotFound("Token not found".into()));
    }
    Ok(())
}

pub fn bearer(headers: &axum::http::HeaderMap, config: &Config) -> AppResult<Claims> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            AppError::Unauthorized(
                "🚪 You are not logged in. Please sign in to continue! 🔐".into(),
            )
        })?;
    verify_access(config, token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_and_refresh_tokens_use_different_secrets() {
        let cfg = Config {
            port: 3000,
            database_url: String::new(),
            redis_url: String::new(),
            access_jwt_secret: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            refresh_jwt_secret: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            public_base_url: String::new(),
            frontend_origins: vec![],
            minio_endpoint: String::new(),
            minio_access_key: String::new(),
            minio_secret_key: String::new(),
            minio_bucket: String::new(),
            minio_region: String::new(),
            production: false,
        };
        let access = create_access(&cfg, "user", "role").unwrap();
        assert!(verify_access(&cfg, &access).is_ok());
        assert!(verify_refresh(&cfg, &access).is_err());
        let refresh = sign(
            &cfg,
            &uuid::Uuid::new_v4().to_string(),
            &uuid::Uuid::new_v4().to_string(),
            true,
        )
        .unwrap();
        assert!(refresh.len() <= 255, "JWT must fit legacy VARCHAR(255)");
    }
}
