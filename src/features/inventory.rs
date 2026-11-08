use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::get,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    authorization,
    catalog_common::{collection, detail, timestamps},
};
use crate::{
    common::{Pagination, not_found, parse_uuid, require_changed_update, success},
    entity::inventory,
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    quantity: Option<i32>,
    reserved_quantity: Option<i32>,
    is_active: Option<bool>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/product/{id}", get(by_product))
        .route("/{id}", get(one).patch(update))
}

pub fn body(m: inventory::Model) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    json!({"id":m.id,"productId":m.product_id,"quantity":m.quantity,"reservedQuantity":m.reserved_quantity,"isActive":m.is_active,"createdAt":created_at,"updatedAt":updated_at})
}

async fn list(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = q.checked()?;
    let total = inventory::Entity::find()
        .count(&state.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("Inventory"));
    }
    let data = inventory::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Inventory",
        data.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}

async fn one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let m = inventory::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Inventory"))?;
    Ok(Json(detail("Inventory", body(m))))
}

async fn by_product(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let m = inventory::Entity::find()
        .filter(inventory::Column::ProductId.eq(parse_uuid(&id)?))
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Inventory"))?;
    Ok(Json(detail("Inventory", body(m))))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Update>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_admin()?;
    let m = inventory::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Inventory"))?;
    require_changed_update(&[
        ("quantity", body.quantity.map(|v| v == m.quantity)),
        (
            "reservedQuantity",
            body.reserved_quantity.map(|v| v == m.reserved_quantity),
        ),
        ("isActive", body.is_active.map(|v| v == m.is_active)),
    ])?;
    // Original InventoryService validates only that submitted fields changed;
    // it does not enforce stock/reservation limits on PATCH.
    let mut a: inventory::ActiveModel = m.into();
    if let Some(v) = body.quantity {
        a.quantity = Set(v);
    }
    if let Some(v) = body.reserved_quantity {
        a.reserved_quantity = Set(v);
    }
    if let Some(v) = body.is_active {
        a.is_active = Set(v);
    }
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&state.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Inventory")))
}
