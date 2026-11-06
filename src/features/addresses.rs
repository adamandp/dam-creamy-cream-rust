use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;
use validator::Validate;

use super::session;
use crate::{
    common::{Pagination, date_time, not_found, paging, parse_uuid, success, success_verb},
    entity::{address, user},
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAddressBody {
    pub user_id: Uuid,
    #[validate(length(min = 1, max = 100))]
    pub recipient_name: String,
    #[validate(length(min = 4, max = 100))]
    pub recipient_phone_number: String,
    #[validate(length(min = 1, max = 100))]
    pub province: String,
    #[validate(length(min = 1, max = 100))]
    pub city: String,
    #[validate(length(min = 1, max = 100))]
    pub subdistrict: String,
    #[validate(length(min = 1, max = 100))]
    pub village: String,
    #[validate(length(max = 100))]
    pub postal_code: String,
    #[validate(length(min = 1, max = 100))]
    pub address: String,
    pub is_primary: bool,
}

#[derive(Deserialize, Default, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAddressBody {
    pub user_id: Option<Uuid>,
    #[validate(length(min = 1, max = 100))]
    pub recipient_name: Option<String>,
    #[validate(length(min = 4, max = 100))]
    pub recipient_phone_number: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub province: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub city: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub subdistrict: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub village: Option<String>,
    #[validate(length(max = 100))]
    pub postal_code: Option<String>,
    #[validate(length(min = 1, max = 100))]
    pub address: Option<String>,
    pub is_primary: Option<bool>,
}

fn validate_recipient_name(value: &str) -> AppResult<()> {
    if !value
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c.is_whitespace())
    {
        return Err(AppError::BadRequest(
            "recipientName must contain only letters and spaces".into(),
        ));
    }
    Ok(())
}

fn validate_recipient_phone(value: &str) -> AppResult<()> {
    if !(10..=15).contains(&value.len()) || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(AppError::BadRequest(
            "recipientPhoneNumber must contain 10–15 digits".into(),
        ));
    }
    Ok(())
}

fn response(model: address::Model) -> Value {
    json!({
        "id": model.id,
        "userId": model.user_id,
        "recipientName": model.recipient_name,
        "recipientPhoneNumber": model.recipient_phone_number,
        "province": model.province,
        "city": model.city,
        "subdistrict": model.subdistrict,
        "village": model.village,
        "postalCode": model.postal_code,
        "address": model.address,
        "isPrimary": model.is_primary,
        "createdAt": date_time(model.created_at),
        "updatedAt": date_time(model.updated_at),
    })
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(find_all).post(create))
        .route("/user", get(find_by_user))
        .route("/{id}", get(find_one).patch(update).delete(remove))
}

async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateAddressBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    body.validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    validate_recipient_name(&body.recipient_name)?;
    validate_recipient_phone(&body.recipient_phone_number)?;
    let tx = state.db.begin().await.map_err(AppError::db)?;
    // Lock the user row to serialize competing primary-address changes.
    if user::Entity::find_by_id(body.user_id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("User"));
    }
    let count = address::Entity::find()
        .filter(address::Column::UserId.eq(body.user_id))
        .count(&tx)
        .await
        .map_err(AppError::db)?;
    let primary = count == 0 || body.is_primary;
    if primary {
        address::Entity::update_many()
            .col_expr(address::Column::IsPrimary, false.into())
            .filter(address::Column::UserId.eq(body.user_id))
            .filter(address::Column::IsPrimary.eq(true))
            .exec(&tx)
            .await
            .map_err(AppError::db)?;
    }
    let now = Utc::now().naive_utc();
    address::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(body.user_id),
        recipient_name: Set(body.recipient_name),
        recipient_phone_number: Set(body.recipient_phone_number),
        province: Set(body.province),
        city: Set(body.city),
        subdistrict: Set(body.subdistrict),
        village: Set(body.village),
        postal_code: Set(body.postal_code),
        address: Set(body.address),
        is_primary: Set(primary),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&tx)
    .await
    .map_err(AppError::db)?;
    tx.commit().await.map_err(AppError::db)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"statusCode": 201, "message": success_verb("created", "Address")})),
    ))
}

async fn find_all(
    State(state): State<AppState>,
    Query(query): Query<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = query.checked()?;
    let total = address::Entity::find()
        .count(&state.db)
        .await
        .map_err(AppError::db)?;
    let items = address::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&state.db)
        .await
        .map_err(AppError::db)?;
    // Legacy AddressService returns an empty collection instead of throwing.
    Ok(Json(json!({
        "message": success_verb("retrieved", "Address"),
        "data": items.into_iter().map(response).collect::<Vec<_>>(),
        "paging": paging(page, limit, total),
    })))
}

async fn find_one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let model = address::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;
    Ok(Json(
        json!({"message": success_verb("retrieved", "Address"), "data": response(model)}),
    ))
}

async fn find_by_user(State(state): State<AppState>, jar: CookieJar) -> AppResult<Json<Value>> {
    let token = jar
        .get("refreshToken")
        .map(|v| v.value())
        .ok_or_else(|| AppError::Unauthorized("Missing refreshToken cookie".into()))?;
    let claims = session::valid_stored_refresh(&state, token).await?;
    let items = address::Entity::find()
        .filter(address::Column::UserId.eq(parse_uuid(&claims.sub)?))
        .all(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(json!({
        "message": success_verb("retrieved", "User Address"),
        "data": items.into_iter().map(response).collect::<Vec<_>>(),
    })))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateAddressBody>,
) -> AppResult<Json<Value>> {
    body.validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    if let Some(name) = body.recipient_name.as_deref() {
        validate_recipient_name(name)?;
    }
    if let Some(phone) = body.recipient_phone_number.as_deref() {
        validate_recipient_phone(phone)?;
    }
    let model = address::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;
    let mut active: address::ActiveModel = model.into();
    if let Some(v) = body.user_id {
        active.user_id = Set(v);
    }
    if let Some(v) = body.recipient_name {
        active.recipient_name = Set(v);
    }
    if let Some(v) = body.recipient_phone_number {
        active.recipient_phone_number = Set(v);
    }
    if let Some(v) = body.province {
        active.province = Set(v);
    }
    if let Some(v) = body.city {
        active.city = Set(v);
    }
    if let Some(v) = body.subdistrict {
        active.subdistrict = Set(v);
    }
    if let Some(v) = body.village {
        active.village = Set(v);
    }
    if let Some(v) = body.postal_code {
        active.postal_code = Set(v);
    }
    if let Some(v) = body.address {
        active.address = Set(v);
    }
    if let Some(v) = body.is_primary {
        active.is_primary = Set(v);
    }
    active.updated_at = Set(Utc::now().naive_utc());
    active.update(&state.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Address")))
}

async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let model = address::Entity::find_by_id(id)
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;
    user::Entity::find_by_id(model.user_id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    let total = address::Entity::find()
        .filter(address::Column::UserId.eq(model.user_id))
        .count(&tx)
        .await
        .map_err(AppError::db)?;
    if total <= 1 {
        return Err(AppError::Conflict("❌ Cannot delete Address! At least one Address is required to keep the system functional. 🔒".into()));
    }
    address::Entity::delete_by_id(id)
        .exec(&tx)
        .await
        .map_err(AppError::db)?;
    tx.commit().await.map_err(AppError::db)?;
    Ok(Json(success("deleted", "Address")))
}
