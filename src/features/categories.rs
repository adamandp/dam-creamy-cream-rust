use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::Utc;
use redis::AsyncCommands;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, EntityTrait, PaginatorTrait, QueryOrder, QuerySelect,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    authorization,
    catalog_common::{collection, detail, reject_no_change, require_text, timestamps},
};
use crate::{
    common::{Pagination, not_found, parse_uuid, success},
    entity::category,
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Create {
    name: String,
    description: Option<String>,
    is_active: bool,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    name: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    description: Option<Option<String>>,
    is_active: Option<bool>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(one).patch(update).delete(remove))
}

pub fn body(m: category::Model) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    json!({"id":m.id,"name":m.name,"description":m.description,"isActive":m.is_active,"createdAt":created_at,"updatedAt":updated_at})
}

async fn invalidate(state: &AppState) {
    // Versioned cache keys make invalidation O(1) and avoid blocking Redis with KEYS.
    // Older versions expire naturally after their one-hour TTL.
    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await {
        let _: redis::RedisResult<i64> = conn.incr("categories:version", 1).await;
    }
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Create>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_admin()?;
    require_text(&body.name, 1, 100, "name")?;
    if let Some(d) = &body.description {
        require_text(d, 1, 255, "description")?;
    }
    let now = Utc::now().naive_utc();
    category::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set(body.name),
        description: Set(body.description),
        is_active: Set(body.is_active),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .map_err(AppError::db)?;
    invalidate(&state).await;
    Ok((StatusCode::CREATED, Json(success("created", "category"))))
}

async fn list(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = query.checked()?;
    let mut version = 0_i64;
    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await
        && let Ok(Some(v)) = conn.get::<_, Option<i64>>("categories:version").await
    {
        version = v;
    }
    let key = format!("categories:{version}:{page}:{limit}");
    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await {
        let cached: redis::RedisResult<Option<String>> = conn.get(&key).await;
        if let Ok(Some(value)) = cached
            && let Ok(result) = serde_json::from_str::<Value>(&value)
        {
            return Ok(Json(result));
        }
    }
    let total = category::Entity::find()
        .count(&state.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("category"));
    }
    let items = category::Entity::find()
        .order_by_asc(category::Column::CreatedAt)
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&state.db)
        .await
        .map_err(AppError::db)?;
    let response = collection(
        "category",
        items.into_iter().map(body).collect(),
        page,
        limit,
        total,
    );
    if let Ok(mut conn) = state.redis.get_multiplexed_async_connection().await {
        let _: redis::RedisResult<()> = conn.set_ex(&key, response.to_string(), 3600).await;
    }
    Ok(Json(response))
}

async fn one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let m = category::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("category"))?;
    Ok(Json(detail("category", body(m))))
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
    let m = category::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("category"))?;
    if let Some(v) = &body.name {
        require_text(v, 1, 100, "name")?;
    }
    if let Some(Some(v)) = &body.description {
        require_text(v, 1, 255, "description")?;
    }
    reject_no_change(&[
        body.name.as_ref().map(|v| *v == m.name),
        body.description.as_ref().map(|v| *v == m.description),
        body.is_active.map(|v| v == m.is_active),
    ])?;
    let mut a: category::ActiveModel = m.into();
    if let Some(v) = body.name {
        a.name = Set(v);
    }
    if let Some(v) = body.description {
        a.description = Set(v);
    }
    if let Some(v) = body.is_active {
        a.is_active = Set(v);
    }
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&state.db).await.map_err(AppError::db)?;
    invalidate(&state).await;
    Ok(Json(success("updated", "category")))
}

async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_admin()?;
    let count = category::Entity::delete_by_id(parse_uuid(&id)?)
        .exec(&state.db)
        .await
        .map_err(AppError::db)?
        .rows_affected;
    if count == 0 {
        return Err(not_found("category"));
    }
    invalidate(&state).await;
    Ok(Json(success("deleted", "category")))
}
