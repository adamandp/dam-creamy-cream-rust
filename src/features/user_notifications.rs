use super::{
    authorization,
    catalog_common::{collection, detail, timestamps},
    notifications,
};
use crate::{
    common::{Pagination, not_found, parse_uuid, success},
    entity::{notification, user, user_notification},
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
    notification_id: Uuid,
    #[serde(default, deserialize_with = "crate::common::coerce_js_bool_optional")]
    is_read: Option<bool>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/me", get(by_me))
        .route("/count/me", get(count_me))
        .route("/notification/{id}", get(by_notification))
        .route("/notification/{id}/count", get(count_notification))
        .route("/{id}", get(one).patch(mark_read))
}
fn body(m: user_notification::Model, notification: Option<notification::Model>) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    let mut v = json!({"id":m.id,"userId":m.user_id,"notificationId":m.notification_id,"isRead":m.is_read,"createdAt":created_at,"updatedAt":updated_at});
    if let Some(n) = notification {
        v["notifications"] = notifications::body(n);
    }
    v
}
async fn detail_row(s: &AppState, m: user_notification::Model) -> AppResult<Value> {
    let notification = notification::Entity::find_by_id(m.notification_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(body(m, notification))
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
        return Err(not_found("User"));
    }
    if notification::Entity::find_by_id(b.notification_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Notification"));
    }
    let now = Utc::now().naive_utc();
    user_notification::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(b.user_id),
        notification_id: Set(b.notification_id),
        is_read: Set(b.is_read.unwrap_or(false)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((
        StatusCode::CREATED,
        Json(success("created", "User Notification")),
    ))
}
async fn page_for(s: &AppState, filter: Option<Uuid>, q: Pagination) -> AppResult<Json<Value>> {
    let (page, limit) = q.checked()?;
    let mut count_query = user_notification::Entity::find();
    let mut query = user_notification::Entity::find();
    if let Some(user_id) = filter {
        count_query = count_query.filter(user_notification::Column::UserId.eq(user_id));
        query = query.filter(user_notification::Column::UserId.eq(user_id));
    }
    let total = count_query.count(&s.db).await.map_err(AppError::db)?;
    if total == 0 {
        return Err(crate::common::plain_not_found("User Notification"));
    }
    let rows = query
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let mut data = Vec::with_capacity(rows.len());
    for m in rows {
        data.push(detail_row(s, m).await?);
    }
    Ok(Json(collection(
        "User Notification",
        data,
        page,
        limit,
        total,
    )))
}
async fn list(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    page_for(&s, None, q).await
}
async fn one(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let m = user_notification::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User Notification"))?;
    authorization::authenticate(&s, &headers)
        .await?
        .require_owner_or_admin(m.user_id)?;
    Ok(Json(detail("User Notification", detail_row(&s, m).await?)))
}
async fn by_me(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let actor = authorization::authenticate(&s, &headers).await?;
    page_for(&s, Some(actor.id), q).await
}
async fn count_me(State(s): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let actor = authorization::authenticate(&s, &headers).await?;
    let count = user_notification::Entity::find()
        .filter(user_notification::Column::UserId.eq(actor.id))
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(detail("User Notification", json!({"count":count}))))
}
async fn by_notification(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let id = parse_uuid(&id)?;
    if notification::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Notification"));
    }
    // Legacy implementation accidentally filters userId rather than notificationId.
    page_for(&s, Some(id), q).await
}
async fn count_notification(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let id = parse_uuid(&id)?;
    if notification::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Notification"));
    }
    let count = user_notification::Entity::find()
        .filter(user_notification::Column::UserId.eq(id))
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(detail("User Notification", json!({"count":count}))))
}
async fn mark_read(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let m = user_notification::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User Notification"))?;
    authorization::authenticate(&s, &headers)
        .await?
        .require_owner_or_admin(m.user_id)?;
    let mut update: user_notification::ActiveModel = m.into();
    update.is_read = Set(true);
    update.updated_at = Set(Utc::now().naive_utc());
    update.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "User Notification")))
}
