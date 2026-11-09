use super::{
    authorization,
    catalog_common::{collection, detail},
};
use crate::{
    common::{Pagination, date_time, parse_uuid, success},
    entity::{product, product_analytic, user},
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Create {
    user_id: Uuid,
    product_id: Uuid,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", axum::routing::post(create))
        .route("/product/{id}", get(by_product))
        .route("/product/{id}/count", get(count_product))
        .route("/user", get(by_user))
}
fn body(m: product_analytic::Model) -> Value {
    json!({"id":m.id,"userId":m.user_id,"productId":m.product_id,"createdAt":date_time(m.created_at)})
}
async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Create>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_owner_or_admin(b.user_id)?;
    if user::Entity::find_by_id(b.user_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(crate::common::not_found("User"));
    }
    if product::Entity::find_by_id(b.product_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(crate::common::not_found("Product"));
    }
    product_analytic::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(b.user_id),
        product_id: Set(b.product_id),
        created_at: Set(Utc::now().naive_utc()),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((
        StatusCode::CREATED,
        Json(success("created", "Product Analytic")),
    ))
}
async fn by_product(
    State(s): State<AppState>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let (page, limit) = q.checked()?;
    // Nest counts the entire analytics table for this response, not the product filter.
    let total = product_analytic::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(crate::common::plain_not_found("Product Analytic"));
    }
    let rows = product_analytic::Entity::find()
        .filter(product_analytic::Column::ProductId.eq(id))
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Product Analytic",
        rows.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}
async fn by_user(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let actor = authorization::authenticate(&s, &headers).await?;
    let (page, limit) = q.checked()?;
    // Same legacy global count as findByProduct.
    let total = product_analytic::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(crate::common::plain_not_found("Product Analytic"));
    }
    let rows = product_analytic::Entity::find()
        .filter(product_analytic::Column::UserId.eq(actor.id))
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Product Analytic",
        rows.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}
async fn count_product(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let count = product_analytic::Entity::find()
        .filter(product_analytic::Column::ProductId.eq(parse_uuid(&id)?))
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(detail("Product Analytic", json!({"count":count}))))
}
