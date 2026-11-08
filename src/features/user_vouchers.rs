use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    authorization,
    catalog_common::{collection, detail, parse_datetime, timestamps},
    vouchers,
};
use crate::{
    common::{Pagination, date_time, not_found, parse_uuid, success},
    entity::{
        user,
        user_voucher::{self, UserVoucherStatus},
        voucher,
    },
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Create {
    user_id: Uuid,
    voucher_id: Uuid,
    claimed_at: String,
    expired_at: String,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/user", get(by_user))
        .route("/voucher/{id}", get(by_voucher))
        .route("/{id}", get(one).patch(use_voucher))
}
pub fn body(m: user_voucher::Model) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    json!({"id":m.id,"userId":m.user_id,"voucherId":m.voucher_id,"status":m.status,"claimedAt":date_time(m.claimed_at),"expiredAt":date_time(m.expired_at),"createdAt":created_at,"updatedAt":updated_at})
}

async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Create>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let actor = authorization::authenticate(&s, &headers).await?;
    actor.require_owner_or_admin(b.user_id)?;
    if user::Entity::find_by_id(b.user_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("User"));
    }
    if voucher::Entity::find_by_id(b.voucher_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Voucher"));
    }
    let claimed = parse_datetime(&b.claimed_at)?;
    let expired = parse_datetime(&b.expired_at)?;
    // The NestJS CreateUserVoucherDto does not require expiredAt >= claimedAt.
    let now = Utc::now().naive_utc();
    user_voucher::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(b.user_id),
        voucher_id: Set(b.voucher_id),
        status: Set(UserVoucherStatus::Claimed),
        claimed_at: Set(claimed),
        expired_at: Set(expired),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((
        StatusCode::CREATED,
        Json(success("created", "User Voucher")),
    ))
}
async fn list(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let (page, limit) = q.checked()?;
    let total = user_voucher::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(crate::common::plain_not_found("User Voucher"));
    }
    let data = user_voucher::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "User Voucher",
        data.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}
async fn one(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let m = user_voucher::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User Voucher"))?;
    authorization::authenticate(&s, &headers)
        .await?
        .require_owner_or_admin(m.user_id)?;
    Ok(Json(detail("User Voucher", body(m))))
}
async fn by_user(State(s): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let actor = authorization::authenticate(&s, &headers).await?;
    let vouchers_for_user = user_voucher::Entity::find()
        .filter(user_voucher::Column::UserId.eq(actor.id))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let mut data = Vec::with_capacity(vouchers_for_user.len());
    for item in vouchers_for_user {
        let voucher_data = voucher::Entity::find_by_id(item.voucher_id)
            .one(&s.db)
            .await
            .map_err(AppError::db)?;
        let mut result = body(item);
        result["vouchers"] = voucher_data.map(vouchers::body).unwrap_or(Value::Null);
        data.push(result);
    }
    Ok(Json(detail("User Voucher", json!(data))))
}
async fn by_voucher(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let data = user_voucher::Entity::find()
        .filter(user_voucher::Column::VoucherId.eq(parse_uuid(&id)?))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(detail(
        "User Voucher",
        json!(data.into_iter().map(body).collect::<Vec<_>>()),
    )))
}
async fn use_voucher(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let m = user_voucher::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User Voucher"))?;
    authorization::authenticate(&s, &headers)
        .await?
        .require_owner_or_admin(m.user_id)?;
    let mut a: user_voucher::ActiveModel = m.into();
    a.status = Set(UserVoucherStatus::Used);
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "User Voucher")))
}
