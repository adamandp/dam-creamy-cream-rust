use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
    routing::get,
};
use chrono::{NaiveDate, Utc};
use regex::Regex;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, DbBackend,
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Statement, TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::LazyLock;
use uuid::Uuid;
use validator::Validate;

use super::{auth, authorization};
use crate::{
    common::{
        Pagination, date, date_time, not_found, paging, parse_uuid, require_changed_update,
        success, success_verb,
    },
    entity::{cart, role, user, user_token},
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
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

fn current_image_url(
    stored: Option<String>,
    user_id: Uuid,
    public_base_url: &str,
) -> Option<String> {
    match stored {
        Some(url) => {
            if let Some(key) = persisted_image_key(&url, user_id) {
                let asset = key.rsplit('/').next()?;
                Some(format!("{public_base_url}/media/users/{user_id}/{asset}"))
            } else {
                // Legacy external image URLs remain untouched.
                Some(url)
            }
        }
        None => None,
    }
}

fn body(model: user::Model, public_base_url: &str) -> Value {
    let image_url = current_image_url(model.image_url, model.id, public_base_url);
    json!({
        "id": model.id,
        "roleId": model.role_id,
        "username": model.username,
        "email": model.email,
        "phoneNumber": model.phone_number,
        "fullName": model.full_name,
        "birthDate": model.birth_date.map(date),
        "imageUrl": image_url,
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
        .or_else(|_| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|date| date.with_timezone(&Utc).date_naive())
        })
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
    headers: HeaderMap,
    ApiQuery(query): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_admin()?;
    let (page, limit) = query.checked()?;
    let total = user::Entity::find()
        .count(&state.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("User"));
    }
    let rows = user::Entity::find()
        .order_by_asc(user::Column::CreatedAt)
        .order_by_asc(user::Column::Id)
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(json!({
        "message": success_verb("retrieved", "User"),
        "data": rows.into_iter().map(|row| body(row, &state.config.public_base_url)).collect::<Vec<_>>(),
        "paging": paging(page, limit, total),
    })))
}

async fn find_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    authorization::authenticate(&state, &headers)
        .await?
        .require_owner_or_admin(id)?;
    let row = user::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    Ok(Json(
        json!({"message": success_verb("retrieved", "User"), "data": body(row, &state.config.public_base_url)}),
    ))
}

async fn profile(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let actor = authorization::authenticate(&state, &headers).await?;
    let row = user::Entity::find_by_id(actor.id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    let mut profile = body(row, &state.config.public_base_url);
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
    headers: HeaderMap,
    ApiJson(input): ApiJson<UpdateUserBody>,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let actor = authorization::authenticate(&state, &headers).await?;
    actor.require_owner_or_admin(id)?;
    if !actor.is_admin && (input.role_id.is_some() || input.is_active.is_some()) {
        return Err(AppError::Forbidden(
            "Only administrators can change roles or activation status".into(),
        ));
    }
    // Password changes require the existing password via the dedicated endpoint.
    if input.password.is_some() {
        return Err(AppError::BadRequest(
            "Use PUT /users/{id}/password to change a password".into(),
        ));
    }
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let row = user::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    if input.role_id.is_some() && !actor.is_super_admin {
        return Err(AppError::Forbidden(
            "Only super administrators can assign roles".into(),
        ));
    }
    if actor.id == id && (input.role_id.is_some() || input.is_active.is_some()) {
        return Err(AppError::Forbidden(
            "Cannot change your own role or activation status".into(),
        ));
    }
    // An administrator must not be able to deactivate, edit or remove a
    // super-administrator account, even through non-role fields.
    if !actor.is_super_admin && actor.id != id {
        let target_role = role::Entity::find_by_id(row.role_id)
            .one(&tx)
            .await
            .map_err(AppError::db)?;
        if target_role.is_some_and(|role| role.name == "super-admin") {
            return Err(AppError::Forbidden(
                "Cannot modify a super administrator".into(),
            ));
        }
    }
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
    let requested_birth_date = input
        .birth_date
        .as_ref()
        .map(|value| value.as_deref().map(parse_birth_date).transpose())
        .transpose()?;
    require_changed_update(&[
        ("roleId", input.role_id.as_ref().map(|v| v == &row.role_id)),
        (
            "username",
            input.username.as_ref().map(|v| v == &row.username),
        ),
        ("email", input.email.as_ref().map(|v| v == &row.email)),
        (
            "phoneNumber",
            input.phone_number.as_ref().map(|v| v == &row.phone_number),
        ),
        (
            "fullName",
            input.full_name.as_ref().map(|v| v == &row.full_name),
        ),
        (
            "birthDate",
            requested_birth_date.as_ref().map(|v| v == &row.birth_date),
        ),
        ("gender", input.gender.as_ref().map(|v| v == &row.gender)),
        (
            "isActive",
            input.is_active.as_ref().map(|v| v == &row.is_active),
        ),
    ])?;
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
    if let Some(v) = requested_birth_date {
        active.birth_date = Set(v);
    }
    active.updated_at = Set(Utc::now().naive_utc());
    active.update(&tx).await.map_err(AppError::db)?;
    // Disabling an account or changing its role must also invalidate its
    // existing refresh cookies. Otherwise a disabled-then-enabled account
    // could silently resurrect previously issued sessions.
    if input.role_id.is_some() || input.is_active == Some(false) {
        user_token::Entity::delete_many()
            .filter(user_token::Column::UserId.eq(id))
            .exec(&tx)
            .await
            .map_err(AppError::db)?;
    }
    tx.commit().await.map_err(AppError::db)?;
    Ok(Json(success("updated", "User")))
}

async fn update_password(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<PasswordBody>,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let actor = authorization::authenticate(&state, &headers).await?;
    if actor.id != id {
        return Err(AppError::Forbidden(
            "Cannot change another user's password".into(),
        ));
    }
    validate_password(&input.exsiting_password)?;
    validate_password(&input.new_password)?;
    let row = user::Entity::find_by_id(id)
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
    let original_hash = row.password.clone();
    let replacement_hash = auth::hash_password(input.new_password).await?;
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let locked = user::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    if locked.password != original_hash {
        return Err(AppError::Conflict(
            "Password changed during this request; please retry".into(),
        ));
    }
    let mut active: user::ActiveModel = locked.into();
    active.password = Set(replacement_hash);
    active.updated_at = Set(Utc::now().naive_utc());
    active.update(&tx).await.map_err(AppError::db)?;
    user_token::Entity::delete_many()
        .filter(user_token::Column::UserId.eq(id))
        .exec(&tx)
        .await
        .map_err(AppError::db)?;
    tx.commit().await.map_err(AppError::db)?;
    Ok(Json(success("updated", "User Password")))
}

async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let actor = authorization::authenticate(&state, &headers).await?;
    actor.require_owner_or_admin(id)?;
    if actor.is_super_admin && actor.id == id {
        // Retain an administrative recovery path even when this is the only
        // remaining super-administrator account.
        return Err(AppError::Forbidden(
            "Super administrators cannot delete their own account".into(),
        ));
    }
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let row = user::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    if !actor.is_super_admin {
        let target_role = role::Entity::find_by_id(row.role_id)
            .one(&tx)
            .await
            .map_err(AppError::db)?;
        if target_role.is_some_and(|role| role.name == "super-admin") {
            return Err(AppError::Forbidden(
                "Cannot delete a super administrator".into(),
            ));
        }
    }
    // Orders and their addresses are financial history, so retaining them is
    // more important than making a destructive deletion silently succeed.
    let historical_orders = tx
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT 1 FROM orders WHERE user_id = $1 OR address_id IN (SELECT id FROM addresses WHERE user_id = $1) LIMIT 1",
            [id.into()],
        ))
        .await
        .map_err(AppError::db)?;
    if historical_orders.is_some() {
        return Err(AppError::Conflict(
            "Account has order history and cannot be permanently deleted".into(),
        ));
    }
    // The original Prisma schema uses RESTRICT for all user dependencies.
    // Remove non-financial associations in child-first order, transactionally.
    for statement in [
        "DELETE FROM cart_items WHERE cart_id IN (SELECT id FROM carts WHERE user_id = $1)",
        "DELETE FROM carts WHERE user_id = $1",
        "DELETE FROM user_tokens WHERE user_id = $1",
        "DELETE FROM addresses WHERE user_id = $1",
        "DELETE FROM reviews WHERE user_id = $1",
        "DELETE FROM user_vouchers WHERE user_id = $1",
        "DELETE FROM product_analytics WHERE user_id = $1",
        "DELETE FROM user_notifications WHERE user_id = $1",
    ] {
        tx.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            statement,
            [id.into()],
        ))
        .await
        .map_err(AppError::db)?;
    }
    let previous_key = row
        .image_url
        .as_deref()
        .and_then(|value| persisted_image_key(value, id));
    let result = user::Entity::delete_by_id(id)
        .exec(&tx)
        .await
        .map_err(AppError::db)?;
    if result.rows_affected == 0 {
        return Err(not_found("User"));
    }
    tx.commit().await.map_err(AppError::db)?;
    if let Some(key) = previous_key
        && let Err(err) = state.storage.delete_image(&key).await
    {
        tracing::warn!(error = %err, object_key = key, "deleted user's image cleanup failed");
    }
    Ok(Json(success("deleted", "User")))
}

async fn authenticate_owner(state: &AppState, headers: &HeaderMap, id: Uuid) -> AppResult<()> {
    let actor = authorization::authenticate(state, headers).await?;
    if actor.id != id {
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
    ApiJson(input): ApiJson<PresignBody>,
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
    // A short-lived staging key prevents clients from overwriting the finalized
    // image using the still-valid presigned PUT URL.
    let key = format!("pending/users/{id}/{}", Uuid::new_v4());
    let url = state
        .storage
        .presign_put(&key, &input.mime_type, input.file_size)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "data": {"uploadUrl": url, "key": key, "expiresIn": 600,
            "headers": {"Content-Type": input.mime_type}},
        })),
    ))
}

async fn confirm_image(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<ConfirmBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = parse_uuid(&id)?;
    authenticate_owner(&state, &headers, id).await?;
    let prefix = format!("pending/users/{id}/");
    let asset_id = input
        .key
        .strip_prefix(&prefix)
        .ok_or_else(|| AppError::BadRequest("Invalid image object key".into()))?;
    parse_uuid(asset_id)?;
    let image_url = format!(
        "{}/media/users/{id}/{asset_id}",
        state.config.public_base_url
    );
    if image_url.len() > 255 {
        return Err(AppError::BadRequest(
            "PUBLIC_BASE_URL is too long for image_url column".into(),
        ));
    }
    let final_key = format!("users/{id}/{asset_id}");
    let write_result: AppResult<Option<String>> = async {
        let tx = state.db.begin().await.map_err(AppError::db)?;
        // Lock before checking the stage object, so a concurrent retry cannot
        // observe the old image then fail after the first request removes the
        // staging key. The copy and DB update share the same serialized lock.
        let row = user::Entity::find_by_id(id)
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| not_found("User"))?;
        if row
            .image_url
            .as_deref()
            .and_then(|value| persisted_image_key(value, id))
            .as_deref()
            == Some(final_key.as_str())
        {
            tx.commit().await.map_err(AppError::db)?;
            return Ok(None);
        }
        let etag = state.storage.verify_image(&input.key).await?;
        let previous_key = row
            .image_url
            .as_deref()
            .and_then(|value| persisted_image_key(value, id));
        state
            .storage
            .copy_verified_image(&input.key, &final_key, &etag)
            .await?;
        let mut active: user::ActiveModel = row.into();
        active.image_url = Set(Some(image_url.clone()));
        active.updated_at = Set(Utc::now().naive_utc());
        active.update(&tx).await.map_err(AppError::db)?;
        tx.commit().await.map_err(AppError::db)?;
        Ok(previous_key)
    }
    .await;
    let previous_key = match write_result {
        Ok(key) => key,
        Err(err) => {
            if let Err(cleanup_err) = delete_image_if_unreferenced(&state, id, &final_key).await {
                tracing::warn!(
                    error = %cleanup_err,
                    object_key = final_key,
                    "could not safely clean up failed image confirmation"
                );
            }
            return Err(err);
        }
    };
    for obsolete in obsolete_image_keys(input.key, previous_key, &final_key) {
        if let Err(err) = delete_image_if_unreferenced(&state, id, &obsolete).await {
            // The new image is committed, so failing the request would mislead the UI.
            tracing::warn!(error = %err, object_key = obsolete, "MinIO image cleanup failed");
        }
    }
    Ok((StatusCode::CREATED, Json(success("updated", "User Image"))))
}

// Both success and failure cleanup must take the same user-row lock as
// confirm_image. Otherwise a later confirmation can reactivate an older
// permanent key between a pointer check and an obsolete-object deletion.
// If the database is unavailable, leave an orphan for later reconciliation
// rather than risk removing a currently referenced object.
pub(crate) async fn delete_image_if_unreferenced(
    state: &AppState,
    user_id: Uuid,
    final_key: &str,
) -> AppResult<()> {
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let account = user::Entity::find_by_id(user_id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?;
    let is_active_image = account
        .as_ref()
        .and_then(|account| account.image_url.as_deref())
        .and_then(|url| persisted_image_key(url, user_id))
        .is_some_and(|key| key == final_key);
    if !is_active_image {
        state.storage.delete_image(final_key).await?;
    }
    tx.commit().await.map_err(AppError::db)?;
    Ok(())
}

fn obsolete_image_keys(pending: String, previous: Option<String>, current: &str) -> Vec<String> {
    [Some(pending), previous]
        .into_iter()
        .flatten()
        .filter(|key| key != current)
        .collect()
}

fn persisted_image_key(url: &str, user_id: Uuid) -> Option<String> {
    let marker = format!("/media/users/{user_id}/");
    let asset_id = url.rsplit_once(&marker)?.1;
    Uuid::parse_str(asset_id).ok()?;
    Some(format!("users/{user_id}/{asset_id}"))
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn only_owned_minio_urls_are_candidates_for_cleanup() {
        let user = Uuid::new_v4();
        let asset = Uuid::new_v4();
        assert_eq!(
            persisted_image_key(
                &format!("https://api.example.com/media/users/{user}/{asset}"),
                user
            ),
            Some(format!("users/{user}/{asset}"))
        );
        assert_eq!(
            persisted_image_key("https://res.cloudinary.com/old.png", user),
            None
        );
        assert_eq!(
            persisted_image_key(
                &format!(
                    "https://api.example.com/media/users/{}/{asset}",
                    Uuid::new_v4()
                ),
                user
            ),
            None
        );
    }

    #[test]
    fn existing_minio_images_follow_the_current_api_origin() {
        let user = Uuid::new_v4();
        let asset = Uuid::new_v4();
        let old = format!("https://old.example.com/media/users/{user}/{asset}");
        assert_eq!(
            current_image_url(Some(old), user, "https://new.example.com"),
            Some(format!(
                "https://new.example.com/media/users/{user}/{asset}"
            ))
        );
        assert_eq!(
            current_image_url(
                Some("https://res.cloudinary.com/legacy.png".into()),
                user,
                "https://new.example.com"
            ),
            Some("https://res.cloudinary.com/legacy.png".into())
        );
    }

    #[test]
    fn duplicate_image_confirmation_never_deletes_current_image() {
        let pending = "pending/users/owner/asset".to_owned();
        let current = "users/owner/asset".to_owned();
        let previous = Some(current.clone());
        assert_eq!(
            obsolete_image_keys(pending.clone(), previous, &current),
            vec![pending]
        );
        assert_eq!(
            obsolete_image_keys(
                "pending/another".into(),
                Some("users/previous".into()),
                &current
            ),
            vec!["pending/another", "users/previous"]
        );
    }
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
    let expected = format!("users/{id}/{asset}");
    if row
        .image_url
        .as_deref()
        .and_then(|value| persisted_image_key(value, id))
        .as_deref()
        != Some(expected.as_str())
    {
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
        assert_eq!(
            parse_birth_date("2026-11-07T00:30:00+14:00").unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 6).unwrap(),
            "RFC3339 offsets must be normalized to UTC before writing a DATE column"
        );
    }
}
