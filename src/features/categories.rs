use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::Utc;
use redis::AsyncCommands;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, PaginatorTrait, QuerySelect};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    authorization,
    catalog_common::{collection, detail, require_name, require_text, timestamps},
};
use crate::{
    common::{Pagination, not_found, parse_uuid, require_changed_update, success},
    entity::category,
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Create {
    name: String,
    #[serde(deserialize_with = "crate::common::required_nullable")]
    description: Option<String>,
    is_active: bool,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
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

fn name_validation_issues(name: &str) -> Vec<Value> {
    let mut issues = Vec::new();
    let len = name.chars().count();
    if len < 1 {
        issues.push(json!({"origin":"string","code":"too_small","minimum":1,"inclusive":true,"path":["name"],"message":"🔡 Category Name must be at least 1 characters. Keep going! 💪 {Minimum Length}"}));
    } else if len > 100 {
        issues.push(json!({"origin":"string","code":"too_big","maximum":100,"inclusive":true,"path":["name"],"message":"🚀 Whoa! Category Name is too long. Maximum allowed is 100 characters! ✍️ {Maximum Length}"}));
    }
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic() || c.is_whitespace()) {
        issues.push(json!({"origin":"string","code":"invalid_format","format":"regex","pattern":"/^[A-Za-z\\s]+$/","path":["name"],"message":"Just letters & spaces, no secret codes! 🤨 {Name Only Letters}"}));
    }
    issues
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<Create>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_admin()?;
    let name_issues = name_validation_issues(&body.name);
    if !name_issues.is_empty() {
        return Err(AppError::Validation(name_issues));
    }
    require_name(&body.name, 1, 100, "name")?;
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
    Ok((StatusCode::CREATED, Json(success("created", "category"))))
}

async fn list(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = query.checked()?;
    // Characterization of the NestJS service: one-hour cache, no write
    // invalidation. This intentionally preserves stale reads after mutations.
    let key = format!("categories:{page}:{limit}");
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
        return Err(crate::common::plain_not_found("category"));
    }
    let items = category::Entity::find()
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
        .ok_or_else(|| not_found("Category"))?;
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
        .ok_or_else(|| not_found("Category"))?;
    if let Some(v) = &body.name {
        let name_issues = name_validation_issues(v);
        if !name_issues.is_empty() {
            return Err(AppError::Validation(name_issues));
        }
        require_name(v, 1, 100, "name")?;
    }
    if let Some(Some(v)) = &body.description {
        require_text(v, 1, 255, "description")?;
    }
    require_changed_update(&[
        ("name", body.name.as_ref().map(|v| *v == m.name)),
        (
            "description",
            body.description.as_ref().map(|v| *v == m.description),
        ),
        ("isActive", body.is_active.map(|v| v == m.is_active)),
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
        return Err(not_found("Category"));
    }
    Ok(Json(success("deleted", "category")))
}
