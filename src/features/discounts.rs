use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, EntityTrait, PaginatorTrait, QueryOrder, QuerySelect,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    authorization,
    catalog_common::{
        collection, detail, parse_datetime, reject_no_change, require_positive, require_text,
        timestamps,
    },
};
use crate::{
    common::{Pagination, date_time, not_found, parse_uuid, success},
    entity::discount::{self, DiscountType},
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Create {
    name: String,
    discount_type: DiscountType,
    value: Option<f64>,
    max_applied: i32,
    max_redeemed: i32,
    redeemed_count: i32,
    description: Option<String>,
    product_id: Option<Uuid>,
    quantity: Option<i32>,
    start_date: String,
    end_date: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    name: Option<String>,
    discount_type: Option<DiscountType>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    value: Option<Option<f64>>,
    max_applied: Option<i32>,
    max_redeemed: Option<i32>,
    redeemed_count: Option<i32>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    description: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    product_id: Option<Option<Uuid>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    quantity: Option<Option<i32>>,
    start_date: Option<String>,
    end_date: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(one).patch(update).delete(remove))
}

pub fn body(m: discount::Model) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    json!({"id":m.id,"name":m.name,"discountType":m.discount_type,"value":m.value,"maxApplied":m.max_applied,"maxRedeemed":m.max_redeemed,"redeemedCount":m.redeemed_count,"description":m.description,"productId":m.product_id,"quantity":m.quantity,"startDate":date_time(m.start_date),"endDate":date_time(m.end_date),"createdAt":created_at,"updatedAt":updated_at})
}

struct DiscountRules<'a> {
    name: &'a str,
    value: Option<f64>,
    max_applied: i32,
    max_redeemed: i32,
    redeemed_count: i32,
    quantity: Option<i32>,
    start: chrono::NaiveDateTime,
    end: chrono::NaiveDateTime,
}

fn validate(rules: DiscountRules<'_>) -> AppResult<()> {
    let DiscountRules { name, value, max_applied, max_redeemed, redeemed_count, quantity, start, end } = rules;
    require_text(name, 1, 50, "name")?;
    if let Some(value) = value {
        require_positive(value, 0.0, 1_000_000_000.0, "value")?;
    }
    if max_applied < 0
        || max_redeemed < 0
        || redeemed_count < 0
        || redeemed_count > max_redeemed
        || quantity.is_some_and(|q| q < 0)
        || start > end
    {
        return Err(AppError::BadRequest(
            "Invalid discount limits, quantity or date interval".into(),
        ));
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
        .require_admin()?;
    let start = parse_datetime(&b.start_date)?;
    let end = parse_datetime(&b.end_date)?;
    validate(DiscountRules {
        name: &b.name,
        value: b.value,
        max_applied: b.max_applied,
        max_redeemed: b.max_redeemed,
        redeemed_count: b.redeemed_count,
        quantity: b.quantity,
        start,
        end,
    })?;
    if let Some(d) = &b.description {
        require_text(d, 1, 255, "description")?;
    }
    let now = Utc::now().naive_utc();
    discount::ActiveModel {
        id: Set(Uuid::new_v4()),
        name: Set(b.name),
        discount_type: Set(b.discount_type),
        value: Set(b.value),
        max_applied: Set(b.max_applied),
        max_redeemed: Set(b.max_redeemed),
        redeemed_count: Set(b.redeemed_count),
        description: Set(b.description),
        product_id: Set(b.product_id),
        quantity: Set(b.quantity),
        start_date: Set(start),
        end_date: Set(end),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((StatusCode::CREATED, Json(success("created", "Discount"))))
}

async fn list(
    State(s): State<AppState>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = q.checked()?;
    let total = discount::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    // Legacy DiscountsService accidentally labels empty collection and successful lists as User.
    if total == 0 {
        return Err(not_found("Users"));
    }
    let data = discount::Entity::find()
        .order_by_asc(discount::Column::CreatedAt)
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "User",
        data.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}

async fn one(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let m = discount::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Discount"))?;
    Ok(Json(detail("Discount", body(m))))
}

async fn update(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Update>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let m = discount::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Discount"))?;
    let start = b
        .start_date
        .as_deref()
        .map(parse_datetime)
        .transpose()?
        .unwrap_or(m.start_date);
    let end = b
        .end_date
        .as_deref()
        .map(parse_datetime)
        .transpose()?
        .unwrap_or(m.end_date);
    validate(DiscountRules {
        name: b.name.as_deref().unwrap_or(&m.name),
        value: b.value.unwrap_or(m.value),
        max_applied: b.max_applied.unwrap_or(m.max_applied),
        max_redeemed: b.max_redeemed.unwrap_or(m.max_redeemed),
        redeemed_count: b.redeemed_count.unwrap_or(m.redeemed_count),
        quantity: b.quantity.unwrap_or(m.quantity),
        start,
        end,
    })?;
    if let Some(Some(d)) = &b.description {
        require_text(d, 1, 255, "description")?;
    }
    reject_no_change(&[
        b.name.as_ref().map(|v| v == &m.name),
        b.discount_type.as_ref().map(|v| v == &m.discount_type),
        b.value.as_ref().map(|v| v == &m.value),
        b.max_applied.map(|v| v == m.max_applied),
        b.max_redeemed.map(|v| v == m.max_redeemed),
        b.redeemed_count.map(|v| v == m.redeemed_count),
        b.description.as_ref().map(|v| v == &m.description),
        b.product_id.as_ref().map(|v| v == &m.product_id),
        b.quantity.as_ref().map(|v| v == &m.quantity),
        b.start_date.as_ref().map(|_| start == m.start_date),
        b.end_date.as_ref().map(|_| end == m.end_date),
    ])?;
    let mut a: discount::ActiveModel = m.into();
    if let Some(v) = b.name {
        a.name = Set(v);
    }
    if let Some(v) = b.discount_type {
        a.discount_type = Set(v);
    }
    if let Some(v) = b.value {
        a.value = Set(v);
    }
    if let Some(v) = b.max_applied {
        a.max_applied = Set(v);
    }
    if let Some(v) = b.max_redeemed {
        a.max_redeemed = Set(v);
    }
    if let Some(v) = b.redeemed_count {
        a.redeemed_count = Set(v);
    }
    if let Some(v) = b.description {
        a.description = Set(v);
    }
    if let Some(v) = b.product_id {
        a.product_id = Set(v);
    }
    if let Some(v) = b.quantity {
        a.quantity = Set(v);
    }
    if b.start_date.is_some() {
        a.start_date = Set(start);
    }
    if b.end_date.is_some() {
        a.end_date = Set(end);
    }
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Discount")))
}

async fn remove(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    if discount::Entity::delete_by_id(parse_uuid(&id)?)
        .exec(&s.db)
        .await
        .map_err(AppError::db)?
        .rows_affected
        == 0
    {
        return Err(not_found("Discount"));
    }
    Ok(Json(success("deleted", "Discount")))
}
