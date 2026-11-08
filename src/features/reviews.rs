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
    catalog_common::{collection, detail, require_text, timestamps},
};
use crate::{
    common::{Pagination, date_time, not_found, parse_uuid, require_changed_update, success},
    entity::{product, review, user},
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Create {
    user_id: Uuid,
    product_id: Uuid,
    rating: i32,
    comment: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    rating: Option<i32>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    comment: Option<Option<String>>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/me", get(by_me))
        .route("/product/{id}", get(by_product))
        .route("/{id}", get(one).patch(update).delete(remove))
}
pub fn body(m: review::Model) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    json!({"id":m.id,"userId":m.user_id,"productId":m.product_id,"rating":m.rating,"comment":m.comment,"createdAt":created_at,"updatedAt":updated_at})
}

fn validate(rating: i32, comment: Option<&str>) -> AppResult<()> {
    if !(1..=5).contains(&rating) {
        return Err(AppError::BadRequest("rating must be 1–5".into()));
    }
    if let Some(v) = comment {
        require_text(v, 1, 255, "comment")?;
    }
    Ok(())
}

async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Create>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_owner_or_admin(b.user_id)?;
    validate(b.rating, b.comment.as_deref())?;
    if user::Entity::find_by_id(b.user_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("User"));
    }
    if product::Entity::find_by_id(b.product_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Product"));
    }
    let now = Utc::now().naive_utc();
    review::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(b.user_id),
        product_id: Set(b.product_id),
        rating: Set(b.rating),
        comment: Set(b.comment),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((StatusCode::CREATED, Json(success("created", "Review"))))
}
async fn list(
    State(s): State<AppState>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = q.checked()?;
    let total = review::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("Review"));
    }
    let data = review::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Review",
        data.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}
async fn one(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let m = review::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Review"))?;
    Ok(Json(detail("Review", body(m))))
}
async fn by_me(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let actor = authorization::authenticate(&s, &headers).await?;
    let (page, limit) = q.checked()?;
    let query = review::Entity::find().filter(review::Column::UserId.eq(actor.id));
    // Legacy service paginates this user's reviews but counts ALL reviews.
    let total = review::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("Review"));
    }
    let data = query
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Review",
        data.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}
async fn by_product(
    State(s): State<AppState>,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    if product::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Product"));
    }
    let (page, limit) = q.checked()?;
    let query = review::Entity::find().filter(review::Column::ProductId.eq(id));
    // Legacy service paginates this product's reviews but counts ALL reviews.
    let total = review::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("Review"));
    }
    let rows = query
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        let account = user::Entity::find_by_id(row.user_id)
            .one(&s.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| not_found("User"))?;
        data.push(json!({"id":row.id,"rate":row.rating,"comment":row.comment,"createdAt":date_time(row.created_at),"name":account.username,"imageUrl":account.image_url}));
    }
    Ok(Json(collection("Review", data, page, limit, total)))
}
async fn update(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Update>,
) -> AppResult<Json<Value>> {
    let m = review::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Review"))?;
    authorization::authenticate(&s, &headers)
        .await?
        .require_owner_or_admin(m.user_id)?;
    validate(
        b.rating.unwrap_or(m.rating),
        b.comment
            .as_ref()
            .map(|v| v.as_deref())
            .unwrap_or(m.comment.as_deref()),
    )?;
    require_changed_update(&[
        ("rating", b.rating.map(|v| v == m.rating)),
        ("comment", b.comment.as_ref().map(|v| v == &m.comment)),
    ])?;
    let mut a: review::ActiveModel = m.into();
    if let Some(v) = b.rating {
        a.rating = Set(v);
    }
    if let Some(v) = b.comment {
        a.comment = Set(v);
    }
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Review")))
}
async fn remove(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    let m = review::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Review"))?;
    authorization::authenticate(&s, &headers)
        .await?
        .require_owner_or_admin(m.user_id)?;
    review::Entity::delete_by_id(m.id)
        .exec(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(success("deleted", "Review")))
}
