use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use axum_extra::extract::cookie::CookieJar;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseTransaction,
    DbBackend, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Statement,
    TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;
use validator::Validate;

use super::{authorization, session};
use crate::{
    common::{
        Pagination, date_time, not_found, paging, parse_uuid, require_changed_update, success,
        success_verb,
    },
    entity::{address, user},
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
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
    #[validate(length(max = 50))]
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
    #[validate(length(max = 50))]
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

// Called while holding the corresponding user's row lock. Repairs legacy data with
// multiple primary addresses, and guarantees exactly one primary if any exist.
async fn normalize_primary(
    tx: &DatabaseTransaction,
    user_id: Uuid,
    preferred: Option<Uuid>,
) -> AppResult<()> {
    let addresses = address::Entity::find()
        .filter(address::Column::UserId.eq(user_id))
        .order_by_asc(address::Column::CreatedAt)
        .order_by_asc(address::Column::Id)
        .all(tx)
        .await
        .map_err(AppError::db)?;
    if addresses.is_empty() {
        return Ok(());
    }
    let primary_id = preferred
        .filter(|id| addresses.iter().any(|item| item.id == *id))
        .or_else(|| {
            addresses
                .iter()
                .find(|item| item.is_primary)
                .map(|item| item.id)
        })
        .unwrap_or(addresses[0].id);
    address::Entity::update_many()
        .col_expr(address::Column::IsPrimary, false.into())
        .filter(address::Column::UserId.eq(user_id))
        .filter(address::Column::Id.ne(primary_id))
        .exec(tx)
        .await
        .map_err(AppError::db)?;
    address::Entity::update_many()
        .col_expr(address::Column::IsPrimary, true.into())
        .filter(address::Column::Id.eq(primary_id))
        .exec(tx)
        .await
        .map_err(AppError::db)?;
    Ok(())
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<CreateAddressBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_owner_or_admin(body.user_id)?;
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
    let now = Utc::now().naive_utc();
    let new_id = Uuid::new_v4();
    address::ActiveModel {
        id: Set(new_id),
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
    normalize_primary(&tx, body.user_id, primary.then_some(new_id)).await?;
    tx.commit().await.map_err(AppError::db)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"statusCode": 201, "message": success_verb("created", "Address")})),
    ))
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
    let total = address::Entity::find()
        .count(&state.db)
        .await
        .map_err(AppError::db)?;
    let items = address::Entity::find()
        .order_by_asc(address::Column::CreatedAt)
        .order_by_asc(address::Column::Id)
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

async fn find_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let model = address::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;
    authorization::authenticate(&state, &headers)
        .await?
        .require_owner_or_admin(model.user_id)?;
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
    let user_id = parse_uuid(&claims.sub)?;
    let account = user::Entity::find_by_id(user_id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::Unauthorized("Account no longer exists".into()))?;
    authorization::ensure_active(&state.config, &account)?;
    let items = address::Entity::find()
        .filter(address::Column::UserId.eq(user_id))
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
    headers: HeaderMap,
    ApiJson(body): ApiJson<UpdateAddressBody>,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let actor = authorization::authenticate(&state, &headers).await?;
    body.validate()
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    if let Some(name) = body.recipient_name.as_deref() {
        validate_recipient_name(name)?;
    }
    if let Some(phone) = body.recipient_phone_number.as_deref() {
        validate_recipient_phone(phone)?;
    }
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let initial = address::Entity::find_by_id(id)
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;
    actor.require_owner_or_admin(initial.user_id)?;
    let target_user_id = body.user_id.unwrap_or(initial.user_id);
    if target_user_id != initial.user_id {
        // Moving an address between accounts is an administrative operation.
        actor.require_admin()?;
    }
    let mut lock_ids = vec![initial.user_id, target_user_id];
    lock_ids.sort_unstable();
    lock_ids.dedup();
    let locked_users = user::Entity::find()
        .filter(user::Column::Id.is_in(lock_ids.iter().copied()))
        .order_by_asc(user::Column::Id)
        .lock_exclusive()
        .all(&tx)
        .await
        .map_err(AppError::db)?;
    if locked_users.len() != lock_ids.len() {
        return Err(not_found("User"));
    }
    let model = address::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;
    if model.user_id != initial.user_id {
        return Err(AppError::Conflict(
            "Address changed concurrently; retry".into(),
        ));
    }
    require_changed_update(&[
        ("userId", body.user_id.as_ref().map(|v| v == &model.user_id)),
        (
            "recipientName",
            body.recipient_name
                .as_ref()
                .map(|v| v == &model.recipient_name),
        ),
        (
            "recipientPhoneNumber",
            body.recipient_phone_number
                .as_ref()
                .map(|v| v == &model.recipient_phone_number),
        ),
        ("province", body.province.as_ref().map(|v| v == &model.province)),
        ("city", body.city.as_ref().map(|v| v == &model.city)),
        (
            "subdistrict",
            body.subdistrict.as_ref().map(|v| v == &model.subdistrict),
        ),
        ("village", body.village.as_ref().map(|v| v == &model.village)),
        (
            "postalCode",
            body.postal_code.as_ref().map(|v| v == &model.postal_code),
        ),
        ("address", body.address.as_ref().map(|v| v == &model.address)),
        (
            "isPrimary",
            body.is_primary.as_ref().map(|v| v == &model.is_primary),
        ),
    ])?;
    let moving = target_user_id != initial.user_id;
    if moving {
        // Orders retain their shipping address, so ownership transfers may
        // never change an address already used by historical orders. The
        // exclusive address lock also blocks concurrent FK references.
        let referenced = tx
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT 1 FROM orders WHERE address_id = $1 LIMIT 1",
                [id.into()],
            ))
            .await
            .map_err(AppError::db)?;
        if referenced.is_some() {
            return Err(AppError::Conflict(
                "Cannot transfer an address referenced by an order".into(),
            ));
        }
    }
    let destination_count = if moving {
        let count = address::Entity::find()
            .filter(address::Column::UserId.eq(target_user_id))
            .count(&tx)
            .await
            .map_err(AppError::db)?;
        let source_count = address::Entity::find()
            .filter(address::Column::UserId.eq(initial.user_id))
            .count(&tx)
            .await
            .map_err(AppError::db)?;
        if source_count <= 1 {
            return Err(AppError::Conflict(
                "Cannot move a user's last address to another account".into(),
            ));
        }
        count
    } else {
        0
    };
    let mut active: address::ActiveModel = model.into();
    if moving {
        active.user_id = Set(target_user_id);
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
    } else if moving {
        // Do not steal the destination user's existing primary address.
        active.is_primary = Set(destination_count == 0);
    }
    if moving && destination_count == 0 {
        active.is_primary = Set(true);
    }
    active.updated_at = Set(Utc::now().naive_utc());
    active.update(&tx).await.map_err(AppError::db)?;
    if moving {
        normalize_primary(&tx, initial.user_id, None).await?;
    }
    let preferred = if body.is_primary == Some(true) || (moving && destination_count == 0) {
        Some(id)
    } else if body.is_primary == Some(false) && !moving {
        // Deselecting the primary should promote another address when possible.
        address::Entity::find()
            .filter(address::Column::UserId.eq(target_user_id))
            .filter(address::Column::Id.ne(id))
            .order_by_asc(address::Column::CreatedAt)
            .one(&tx)
            .await
            .map_err(AppError::db)?
            .map(|address| address.id)
    } else {
        None
    };
    normalize_primary(&tx, target_user_id, preferred).await?;
    tx.commit().await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Address")))
}

async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let actor = authorization::authenticate(&state, &headers).await?;
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let initial = address::Entity::find_by_id(id)
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;
    actor.require_owner_or_admin(initial.user_id)?;
    user::Entity::find_by_id(initial.user_id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    let model = address::Entity::find_by_id(id)
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;
    if model.user_id != initial.user_id {
        return Err(AppError::Conflict(
            "Address changed concurrently; retry".into(),
        ));
    }
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
    normalize_primary(&tx, model.user_id, None).await?;
    tx.commit().await.map_err(AppError::db)?;
    Ok(Json(success("deleted", "Address")))
}
