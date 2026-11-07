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
    entity::{
        product,
        voucher::{self, VoucherGroup, VoucherType},
    },
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Create {
    code: String,
    name: String,
    description: Option<String>,
    r#type: VoucherType,
    group: VoucherGroup,
    value: Option<f64>,
    product_id: Option<Uuid>,
    quantity: Option<i32>,
    minimum_purchase: f64,
    is_active: bool,
    start_date: String,
    end_date: String,
    max_redeemed: i32,
    redeemed_count: i32,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    code: Option<String>,
    name: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    description: Option<Option<String>>,
    r#type: Option<VoucherType>,
    group: Option<VoucherGroup>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    value: Option<Option<f64>>,
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
    minimum_purchase: Option<f64>,
    is_active: Option<bool>,
    start_date: Option<String>,
    end_date: Option<String>,
    max_redeemed: Option<i32>,
    redeemed_count: Option<i32>,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(one).patch(update).delete(remove))
}

pub fn body(m: voucher::Model) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    json!({"id":m.id,"code":m.code,"name":m.name,"description":m.description,"type":m.r#type,"group":m.group,"value":m.value,"productId":m.product_id,"quantity":m.quantity,"minimumPurchase":m.minimum_purchase,"isActive":m.is_active,"startDate":date_time(m.start_date),"endDate":date_time(m.end_date),"maxRedeemed":m.max_redeemed,"redeemedCount":m.redeemed_count,"createdAt":created_at,"updatedAt":updated_at})
}
fn validate(
    code: &str,
    name: &str,
    description: Option<&str>,
    value: Option<f64>,
    minimum: f64,
    quantity: Option<i32>,
    max: i32,
    redeemed: i32,
    start: chrono::NaiveDateTime,
    end: chrono::NaiveDateTime,
) -> AppResult<()> {
    require_text(code, 1, 50, "code")?;
    require_text(name, 1, 50, "name")?;
    if let Some(v) = description {
        require_text(v, 1, 255, "description")?;
    }
    if let Some(v) = value {
        require_positive(v, 0.0, 1_000_000_000.0, "value")?;
    }
    require_positive(minimum, 0.0, 1_000_000_000.0, "minimumPurchase")?;
    if max < 0 || redeemed < 0 || redeemed > max || quantity.is_some_and(|q| q < 0) || start > end {
        return Err(AppError::BadRequest(
            "Invalid voucher limits, quantity or date interval".into(),
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
    validate(
        &b.code,
        &b.name,
        b.description.as_deref(),
        b.value,
        b.minimum_purchase,
        b.quantity,
        b.max_redeemed,
        b.redeemed_count,
        start,
        end,
    )?;
    if let Some(id) = b.product_id {
        if product::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .map_err(AppError::db)?
            .is_none()
        {
            return Err(not_found("Product"));
        }
    }
    let now = Utc::now().naive_utc();
    voucher::ActiveModel {
        id: Set(Uuid::new_v4()),
        code: Set(b.code),
        name: Set(b.name),
        description: Set(b.description),
        r#type: Set(b.r#type),
        group: Set(b.group),
        value: Set(b.value),
        product_id: Set(b.product_id),
        quantity: Set(b.quantity),
        minimum_purchase: Set(b.minimum_purchase),
        is_active: Set(b.is_active),
        start_date: Set(start),
        end_date: Set(end),
        max_redeemed: Set(b.max_redeemed),
        redeemed_count: Set(b.redeemed_count),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((StatusCode::CREATED, Json(success("created", "Voucher"))))
}
async fn list(
    State(s): State<AppState>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = q.checked()?;
    let total = voucher::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("Voucher"));
    }
    let data = voucher::Entity::find()
        .order_by_asc(voucher::Column::CreatedAt)
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Voucher",
        data.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}
async fn one(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let m = voucher::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Voucher"))?;
    Ok(Json(detail("Voucher", body(m))))
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
    let m = voucher::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Voucher"))?;
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
    validate(
        b.code.as_deref().unwrap_or(&m.code),
        b.name.as_deref().unwrap_or(&m.name),
        b.description
            .as_ref()
            .map(|v| v.as_deref())
            .unwrap_or(m.description.as_deref()),
        b.value.unwrap_or(m.value),
        b.minimum_purchase.unwrap_or(m.minimum_purchase),
        b.quantity.unwrap_or(m.quantity),
        b.max_redeemed.unwrap_or(m.max_redeemed),
        b.redeemed_count.unwrap_or(m.redeemed_count),
        start,
        end,
    )?;
    if let Some(Some(id)) = b.product_id {
        if product::Entity::find_by_id(id)
            .one(&s.db)
            .await
            .map_err(AppError::db)?
            .is_none()
        {
            return Err(not_found("Product"));
        }
    }
    reject_no_change(&[
        b.code.as_ref().map(|v| v == &m.code),
        b.name.as_ref().map(|v| v == &m.name),
        b.description.as_ref().map(|v| v == &m.description),
        b.r#type.as_ref().map(|v| v == &m.r#type),
        b.group.as_ref().map(|v| v == &m.group),
        b.value.as_ref().map(|v| v == &m.value),
        b.product_id.as_ref().map(|v| v == &m.product_id),
        b.quantity.as_ref().map(|v| v == &m.quantity),
        b.minimum_purchase.map(|v| v == m.minimum_purchase),
        b.is_active.map(|v| v == m.is_active),
        b.start_date.as_ref().map(|_| start == m.start_date),
        b.end_date.as_ref().map(|_| end == m.end_date),
        b.max_redeemed.map(|v| v == m.max_redeemed),
        b.redeemed_count.map(|v| v == m.redeemed_count),
    ])?;
    let mut a: voucher::ActiveModel = m.into();
    if let Some(v) = b.code {
        a.code = Set(v);
    }
    if let Some(v) = b.name {
        a.name = Set(v);
    }
    if let Some(v) = b.description {
        a.description = Set(v);
    }
    if let Some(v) = b.r#type {
        a.r#type = Set(v);
    }
    if let Some(v) = b.group {
        a.group = Set(v);
    }
    if let Some(v) = b.value {
        a.value = Set(v);
    }
    if let Some(v) = b.product_id {
        a.product_id = Set(v);
    }
    if let Some(v) = b.quantity {
        a.quantity = Set(v);
    }
    if let Some(v) = b.minimum_purchase {
        a.minimum_purchase = Set(v);
    }
    if let Some(v) = b.is_active {
        a.is_active = Set(v);
    }
    if b.start_date.is_some() {
        a.start_date = Set(start);
    }
    if b.end_date.is_some() {
        a.end_date = Set(end);
    }
    if let Some(v) = b.max_redeemed {
        a.max_redeemed = Set(v);
    }
    if let Some(v) = b.redeemed_count {
        a.redeemed_count = Set(v);
    }
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Voucher")))
}
async fn remove(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let m = voucher::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Voucher"))?;
    let mut a: voucher::ActiveModel = m.into();
    a.is_active = Set(false);
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("deleted", "Voucher")))
}
