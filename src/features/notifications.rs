use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, PaginatorTrait, QuerySelect};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    authorization,
    catalog_common::{collection, detail, require_text, timestamps},
};
use crate::{
    common::{Pagination, not_found, parse_uuid, require_changed_update, success},
    entity::notification,
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
pub struct Create {
    title: String,
    message: String,
}
#[derive(Default, Deserialize)]
pub struct Update {
    title: Option<String>,
    message: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(one).patch(update).delete(remove))
}
pub fn body(m: notification::Model) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    json!({"id":m.id,"title":m.title,"message":m.message,"createdAt":created_at,"updatedAt":updated_at})
}
fn validate(title: &str, message: &str) -> AppResult<()> {
    require_text(title, 1, 50, "title")?;
    require_text(message, 1, 255, "message")
}
async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Create>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    validate(&b.title, &b.message)?;
    let now = Utc::now().naive_utc();
    notification::ActiveModel {
        id: Set(Uuid::new_v4()),
        title: Set(b.title),
        message: Set(b.message),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((
        StatusCode::CREATED,
        Json(success("created", "Notification")),
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
    let total = notification::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(crate::common::plain_not_found("Notification"));
    }
    let rows = notification::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Notification",
        rows.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}
async fn one(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let m = notification::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Notification"))?;
    Ok(Json(detail("Notification", body(m))))
}
async fn update(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiJson(b): ApiJson<Update>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let old = notification::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Notification"))?;
    if let Some(ref title) = b.title {
        require_text(title, 1, 50, "title")?;
    }
    if let Some(ref message) = b.message {
        require_text(message, 1, 255, "message")?;
    }
    require_changed_update(&[
        ("title", b.title.as_ref().map(|v| v == &old.title)),
        ("message", b.message.as_ref().map(|v| v == &old.message)),
    ])?;
    let mut m: notification::ActiveModel = old.into();
    if let Some(v) = b.title {
        m.title = Set(v);
    }
    if let Some(v) = b.message {
        m.message = Set(v);
    }
    m.updated_at = Set(Utc::now().naive_utc());
    m.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Notification")))
}
async fn remove(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let m = notification::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Notification"))?;
    notification::Entity::delete_by_id(m.id)
        .exec(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(success("deleted", "Notification")))
}
