use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use woothee::parser::Parser;

use super::session;
use crate::{
    entity::user,
    error::{AppError, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/dummy-customer-login", post(dummy_login))
        .route("/logout", post(logout))
        .route("/logout-all", post(logout_all))
}

pub async fn verify_password(plain: String, hashed: String) -> AppResult<bool> {
    tokio::task::spawn_blocking(move || bcrypt::verify(plain, &hashed))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(|e| AppError::Internal(e.to_string()))
}

pub async fn hash_password(plain: String) -> AppResult<String> {
    tokio::task::spawn_blocking(move || bcrypt::hash(plain, 10))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(|e| AppError::Internal(e.to_string()))
}

fn browser_info(user_agent: &str, ip: &str) -> String {
    let parser = Parser::new();
    let parsed = parser.parse(user_agent);
    let browser = parsed.as_ref().map_or_else(
        || "Unknown Unknown".to_owned(),
        |value| format!("{} {}", value.name, value.version),
    );
    let os = parsed.as_ref().map_or_else(
        || "Unknown Unknown".to_owned(),
        |value| format!("{} {}", value.os, value.os_version),
    );
    let device = parsed
        .as_ref()
        .map_or("Unknown", |value| match value.category {
            "smartphone" | "mobilephone" => "mobile",
            "tablet" => "tablet",
            _ => "Unknown",
        });
    json!({"ip": ip, "browser": browser, "os": os, "device": device}).to_string()
}

async fn dummy_login(
    State(state): State<AppState>,
    jar: CookieJar,
    headers: HeaderMap,
) -> AppResult<(CookieJar, (StatusCode, Json<Value>))> {
    let user = user::Entity::find()
        .filter(user::Column::Username.eq("DummyUser"))
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::NotFound("DummyUser not found".into()))?;
    if !verify_password("@Password123".into(), user.password.clone()).await? {
        return Err(AppError::Unauthorized(
            "🚫 Login failed. Please check your identifier and password, then try again! 🔍".into(),
        ));
    }
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unkown");
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unkown");
    let browser_info = browser_info(&format!("dummy-{ua}"), ip);
    let old = jar.get("refreshToken").map(|v| v.value());
    let (access, refresh) = session::create_dummy_tokens(
        &state,
        &user.id.to_string(),
        &user.role_id.to_string(),
        old,
        browser_info,
    )
    .await?;
    let cookie = Cookie::build(("refreshToken", refresh))
        .http_only(true)
        .secure(state.config.production)
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::days(7))
        .build();
    Ok((
        jar.add(cookie),
        (
            StatusCode::CREATED,
            Json(json!({
                "message": format!("🔓 Welcome back, {}! You have logged in successfully! 🎉", user.username),
                "data": {"accessToken": access},
            })),
        ),
    ))
}

async fn logout(
    State(state): State<AppState>,
    jar: CookieJar,
) -> AppResult<(CookieJar, (StatusCode, Json<Value>))> {
    let token = jar
        .get("refreshToken")
        .map(|v| v.value())
        .unwrap_or_default()
        .to_owned();
    session::clear_token(&state, &token).await?;
    Ok((
        jar.remove(Cookie::build("refreshToken").path("/").build()),
        (
            StatusCode::CREATED,
            Json(json!({"message": "🎉 You have logged out successfully! see u lateer"})),
        ),
    ))
}

async fn logout_all(
    State(state): State<AppState>,
    jar: CookieJar,
) -> AppResult<(CookieJar, (StatusCode, Json<Value>))> {
    let token = jar
        .get("refreshToken")
        .map(|v| v.value())
        .unwrap_or_default()
        .to_owned();
    session::clear_all(&state, &token).await?;
    Ok((
        jar.remove(Cookie::build("refreshToken").path("/").build()),
        (
            StatusCode::CREATED,
            Json(
                json!({"message": "🎉 You have logged out all sessions successfully! see u lateer"}),
            ),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::browser_info;

    #[test]
    fn stores_parsed_browser_metadata_in_legacy_shape() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        let value: serde_json::Value =
            serde_json::from_str(&browser_info(&format!("dummy-{ua}"), "127.0.0.1")).unwrap();
        assert!(value["browser"].as_str().unwrap().contains("Chrome"));
        assert_eq!(value["ip"], "127.0.0.1");
        assert!(value.get("userAgent").is_none());
    }
}
