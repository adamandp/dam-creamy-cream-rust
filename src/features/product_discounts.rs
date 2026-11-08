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
    catalog_common::{collection, detail, timestamps},
};
use crate::{
    common::{Pagination, not_found, parse_uuid, success},
    entity::{discount, product, product_discount, user},
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Create {
    product_id: Uuid,
    discount_id: Uuid,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/product/{productId}", get(by_product))
        .route("/{id}", get(one).delete(remove))
}
pub fn body(m: product_discount::Model) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    json!({"id":m.id,"productId":m.product_id,"discountId":m.discount_id,"isActive":m.is_active,"createdAt":created_at,"updatedAt":updated_at})
}

async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Create>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    if product::Entity::find_by_id(b.product_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Product"));
    }
    if discount::Entity::find_by_id(b.discount_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Discount"));
    }
    let now = Utc::now().naive_utc();
    product_discount::ActiveModel {
        id: Set(Uuid::new_v4()),
        product_id: Set(b.product_id),
        discount_id: Set(b.discount_id),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((
        StatusCode::CREATED,
        Json(success("created", "Product Discount")),
    ))
}
async fn list(
    State(s): State<AppState>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = q.checked()?;
    // Nest's ProductDiscountsService.findAll counts users, not links.
    // This is a known legacy pagination bug retained for 1:1 parity.
    let total = user::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("Product Discount"));
    }
    let data = product_discount::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Product Discount",
        data.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}
async fn one(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let m = product_discount::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Product Discount"))?;
    Ok(Json(detail("Product Discount", body(m))))
}
async fn by_product(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let data = product_discount::Entity::find()
        .filter(product_discount::Column::ProductId.eq(parse_uuid(&id)?))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(detail(
        "Product Discount",
        json!(data.into_iter().map(body).collect::<Vec<_>>()),
    )))
}
async fn remove(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let m = product_discount::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Product Discount"))?;
    let mut a: product_discount::ActiveModel = m.into();
    a.is_active = Set(false);
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("deleted", "Product Discount")))
}
