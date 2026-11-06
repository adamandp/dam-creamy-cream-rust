use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
    routing::get,
};
use chrono::{NaiveDate, Utc};
use regex::Regex;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, EntityTrait, PaginatorTrait,
    QueryFilter, QuerySelect, TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::LazyLock;
use uuid::Uuid;
use validator::Validate;

use super::{auth, session};
use crate::{
    common::{Pagination, date, date_time, not_found, paging, parse_uuid, success, success_verb},
    entity::{cart, role, user},
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserBody {
    #[validate(length(min = 5, max = 50))]
    pub username: String,
    #[validate(length(min = 1, max = 100))]
    pub full_name: String,
    pub password: String,
    pub role_id: Uuid,
    #[validate(email)]
    pub email: Option<String>,
    pub phone_number: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateUserBody {
    pub role_id: Option<Uuid>,
    pub username: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    pub email: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    pub phone_number: Option<Option<String>>,
    pub password: Option<String>,
    pub full_name: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    pub birth_date: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    pub gender: Option<Option<user::GenderType>>,
    pub is_active: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasswordBody {
    // Matches the misspelled key in the existing NestJS DTO.
    pub exsiting_password: String,
    pub new_password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresignBody {
    pub file_name: String,
    pub file_size: i64,
    pub mime_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmBody {
    pub key: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(find_all))
        .route("/profile", get(profile))
        .route("/{id}", get(find_one).patch(update).delete(remove))
        .route("/{id}/password", axum::routing::put(update_password))
        .route("/{id}/image/presign", axum::routing::post(presign_image))
        .route("/{id}/image/confirm", axum::routing::post(confirm_image))
}

fn body(model: user::Model) -> Value {
    json!({
        "id": model.id,
        "roleId": model.role_id,
        "username": model.username,
        "email": model.email,
        "phoneNumber": model.phone_number,
        "fullName": model.full_name,
        "birthDate": model.birth_date.map(date),
        "imageUrl": model.image_url,
        "gender": model.gender,
        "isActive": model.is_active,
        "createdAt": date_time(model.created_at),
        "updatedAt": date_time(model.updated_at),
    })
}

fn validate_password(value: &str) -> AppResult<()> {
    if value.len() < 8
        || value.len() > 255
        || !value.chars().any(|c| c.is_ascii_lowercase())
        || !value.chars().any(|c| c.is_ascii_uppercase())
        || !value.chars().any(|c| c.is_ascii_digit())
        || !value.chars().any(|c| "!@#$%^&*".contains(c))
    {
        return Err(AppError::BadRequest("Password must be 8–255 characters and contain upper/lowercase letters, a number and a special character".into()));
    }
    Ok(())
}

fn validate_text(value: &str, min: usize, max: usize, field: &str) -> AppResult<()> {
    if value.len() < min || value.len() > max {
        return Err(AppError::BadRequest(format!(
            "{field} must be {min}–{max} characters"
        )));
    }
    Ok(())
}

static EMAIL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$")
        .expect("valid email pattern")
});

fn validate_name(value: &str, min: usize, max: usize, field: &str) -> AppResult<()> {
    validate_text(value, min, max, field)?;
    if !value
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c.is_whitespace())
    {
        return Err(AppError::BadRequest(format!(
            "{field} must contain only letters and spaces"
        )));
    }
    Ok(())
}

fn validate_phone(value: &str) -> AppResult<()> {
    if !(10..=15).contains(&value.len()) || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "Phone number must contain 10–15 digits".into(),
        ));
    }
    Ok(())
}

fn validate_email(value: &str) -> AppResult<()> {
    validate_text(value, 4, 100, "email")?;
    if !EMAIL_PATTERN.is_match(value) {
        return Err(AppError::BadRequest("Invalid email".into()));
    }
    Ok(())
}

fn parse_birth_date(value: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .or_else(|_| chrono::DateTime::parse_from_rfc3339(value).map(|date| date.date_naive()))
        .map_err(|_| AppError::BadRequest("Invalid birthDate".into()))
}

async fn unique_fields(
    state: &AppState,
    username: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    exclude: Option<Uuid>,
) -> AppResult<()> {
    let mut or = Condition::any();
    let mut count = 0;
    if let Some(v) = username {
        or = or.add(user::Column::Username.eq(v));
        count += 1;
    }
    if let Some(v) = email {
        or = or.add(user::Column::Email.eq(v));
        count += 1;
    }
    if let Some(v) = phone {
        or = or.add(user::Column::PhoneNumber.eq(v));
        count += 1;
    }
    if count == 0 {
        return Ok(());
    }
    let mut query = user::Entity::find().filter(or);
    if let Some(id) = exclude {
        query = query.filter(user::Column::Id.ne(id));
    }
    if let Some(existing) = query.one(&state.db).await.map_err(AppError::db)? {
        let field = if username.is_some_and(|v| v == existing.username) {
            "username"
        } else if email.is_some_and(|v| Some(v) == existing.email.as_deref()) {
            "email"
        } else {
            "phoneNumber"
        };
        return Err(AppError::Conflict(format!(
            "⚡ {field} already exists! No duplicates allowed! 😆"
        )));
    }
    Ok(())
}

// Original NestJS exposes this operation as an internal service rather than a POST /users route.
// Keep it available to later modules without adding an HTTP route that didn't exist.
pub async fn create_internal(state: &AppState, input: CreateUserBody) -> AppResult<()> {
    input
        .validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    validate_name(&input.username, 5, 50, "username")?;
    validate_name(&input.full_name, 1, 100, "fullName")?;
    if let Some(ref email) = input.email {
        validate_email(email)?;
    }
    if let Some(ref phone) = input.phone_number {
        validate_phone(phone)?;
    }
    validate_password(&input.password)?;
    if role::Entity::find_by_id(input.role_id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Role"));
    }
    unique_fields(
        state,
        Some(&input.username),
        input.email.as_deref(),
        input.phone_number.as_deref(),
        None,
    )
    .await?;
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let now = Utc::now().naive_utc();
    let user_id = Uuid::new_v4();
    user::ActiveModel {
        id: Set(user_id),
        role_id: Set(input.role_id),
        username: Set(input.username),
        email: Set(input.email),
        phone_number: Set(input.phone_number),
        password: Set(auth::hash_password(input.password).await?),
        full_name: Set(input.full_name),
        birth_date: Set(None),
        image_url: Set(None),
        gender: Set(None),
        is_active: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&tx)
    .await
    .map_err(AppError::db)?;
    cart::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(user_id),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&tx)
    .await
    .map_err(AppError::db)?;
    tx.commit().await.map_err(AppError::db)?;
    Ok(())
}

async fn find_all(
    State(state): State<AppState>,
    Query(query): Query<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = query.checked()?;
    let total = user::Entity::find()
        .count(&state.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("User"));
    }
    let rows = user::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(json!({
        "message": success_verb("retrieved", "User"),
        "data": rows.into_iter().map(body).collect::<Vec<_>>(),
        "paging": paging(page, limit, total),
    })))
}

async fn find_one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let row = user::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    Ok(Json(
        json!({"message": success_verb("retrieved", "User"), "data": body(row)}),
    ))
}

async fn profile(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let claims = session::bearer(&headers, &state.config)?;
    let row = user::Entity::find_by_id(parse_uuid(&claims.sub)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    let mut profile = body(row);
    for name in ["id", "roleId", "isActive", "createdAt", "updatedAt"] {
        if let Some(obj) = profile.as_object_mut() {
            obj.remove(name);
        }
    }
    Ok(Json(
        json!({"message": success_verb("retrieved", "User"), "data": profile}),
    ))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateUserBody>,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let row = user::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    if let Some(role_id) = input.role_id
        && role::Entity::find_by_id(role_id)
            .one(&state.db)
            .await
            .map_err(AppError::db)?
            .is_none()
    {
        return Err(not_found("Role"));
    }
    if let Some(ref name) = input.username {
        validate_name(name, 5, 50, "username")?;
    }
    if let Some(ref name) = input.full_name {
        validate_name(name, 1, 100, "fullName")?;
    }
    if let Some(Some(ref email)) = input.email {
        validate_email(email)?;
    }
    if let Some(Some(ref phone)) = input.phone_number {
        validate_phone(phone)?;
    }
    unique_fields(
        &state,
        input.username.as_deref(),
        input.email.as_ref().and_then(|v| v.as_deref()),
        input.phone_number.as_ref().and_then(|v| v.as_deref()),
        Some(id),
    )
    .await?;
    let mut active: user::ActiveModel = row.into();
    if let Some(v) = input.role_id {
        active.role_id = Set(v);
    }
    if let Some(v) = input.username {
        active.username = Set(v);
    }
    if let Some(v) = input.email {
        active.email = Set(v);
    }
    if let Some(v) = input.phone_number {
        active.phone_number = Set(v);
    }
    if let Some(v) = input.full_name {
        active.full_name = Set(v);
    }
    if let Some(v) = input.is_active {
        active.is_active = Set(v);
    }
    if let Some(v) = input.gender {
        active.gender = Set(v);
    }
    if let Some(v) = input.birth_date {
        active.birth_date = Set(v.map(|v| parse_birth_date(&v)).transpose()?);
    }
    if let Some(v) = input.password {
        validate_password(&v)?;
        active.password = Set(auth::hash_password(v).await?);
    }
    active.updated_at = Set(Utc::now().naive_utc());
    active.update(&state.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "User")))
}

async fn update_password(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<PasswordBody>,
) -> AppResult<Json<Value>> {
    validate_password(&input.exsiting_password)?;
    validate_password(&input.new_password)?;
    let row = user::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    if !auth::verify_password(input.exsiting_password, row.password.clone()).await? {
        return Err(AppError::BadRequest(
            "🔐 Existing password is incorrect. Please try again with the correct one! 🙅‍♂️".into(),
        ));
    }
    if auth::verify_password(input.new_password.clone(), row.password.clone()).await? {
        return Err(AppError::BadRequest("🔁 New password can't be the same as your current password. Try something different! 😉".into()));
    }
    let mut active: user::ActiveModel = row.into();
    active.password = Set(auth::hash_password(input.new_password).await?);
    active.updated_at = Set(Utc::now().naive_utc());
    active.update(&state.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "User Password")))
}

async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let result = user::Entity::delete_by_id(parse_uuid(&id)?)
        .exec(&state.db)
        .await
        .map_err(AppError::db)?;
    if result.rows_affected == 0 {
        return Err(not_found("User"));
    }
    Ok(Json(success("deleted", "User")))
}

async fn authenticate_owner(state: &AppState, headers: &HeaderMap, id: Uuid) -> AppResult<()> {
    let claims = session::bearer(headers, &state.config)?;
    if claims.sub != id.to_string() {
        return Err(AppError::Forbidden(
            "Cannot change another user's image".into(),
        ));
    }
    Ok(())
}

async fn presign_image(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<PresignBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = parse_uuid(&id)?;
    authenticate_owner(&state, &headers, id).await?;
    if user::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("User"));
    }
    if input.file_name.is_empty() || input.file_name.len() > 255 {
        return Err(AppError::BadRequest("Invalid fileName".into()));
    }
    if !(1..=2_097_152).contains(&input.file_size) {
        return Err(AppError::BadRequest(
            "Image size must be at most 2 MiB".into(),
        ));
    }
    if !matches!(
        input.mime_type.as_str(),
        "image/jpeg" | "image/png" | "image/webp"
    ) {
        return Err(AppError::BadRequest(
            "Only JPEG, PNG or WebP images are supported".into(),
        ));
    }
    let key = format!("users/{id}/{}", Uuid::new_v4());
    let url = state
        .storage
        .presign_put(&key, &input.mime_type, input.file_size)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "data": {"uploadUrl": url, "key": key, "expiresIn": 600,
            "headers": {"Content-Type": input.mime_type, "Content-Length": input.file_size}},
        })),
    ))
}

async fn confirm_image(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(input): Json<ConfirmBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = parse_uuid(&id)?;
    authenticate_owner(&state, &headers, id).await?;
    let prefix = format!("users/{id}/");
    let asset_id = input
        .key
        .strip_prefix(&prefix)
        .ok_or_else(|| AppError::BadRequest("Invalid image object key".into()))?;
    parse_uuid(asset_id)?;
    state.storage.verify_image(&input.key).await?;
    let row = user::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    let image_url = format!(
        "{}/media/users/{id}/{asset_id}",
        state.config.public_base_url
    );
    if image_url.len() > 255 {
        return Err(AppError::BadRequest(
            "PUBLIC_BASE_URL is too long for image_url column".into(),
        ));
    }
    let mut active: user::ActiveModel = row.into();
    active.image_url = Set(Some(image_url));
    active.updated_at = Set(Utc::now().naive_utc());
    active.update(&state.db).await.map_err(AppError::db)?;
    Ok((StatusCode::CREATED, Json(success("updated", "User Image"))))
}

pub async fn get_image(
    State(state): State<AppState>,
    Path((id, asset)): Path<(String, String)>,
) -> AppResult<Redirect> {
    let id = parse_uuid(&id)?;
    parse_uuid(&asset)?;
    let row = user::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    let expected = format!("{}/media/users/{id}/{asset}", state.config.public_base_url);
    if row.image_url.as_deref() != Some(expected.as_str()) {
        return Err(not_found("User Image"));
    }
    let url = state
        .storage
        .presign_get(&format!("users/{id}/{asset}"))
        .await?;
    Ok(Redirect::temporary(&url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_rules_match_nest_dto() {
        assert!(validate_password("@Password123").is_ok());
        assert!(validate_password("password123").is_err());
        assert!(validate_password("Password123").is_err());
    }

    #[test]
    fn parse_birth_date_supports_date_and_rfc3339() {
        assert_eq!(
            parse_birth_date("2004-09-26").unwrap(),
            parse_birth_date("2004-09-26T00:00:00.000Z").unwrap()
        );
    }
}
