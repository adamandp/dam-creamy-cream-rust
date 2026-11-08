//! OrdersService is currently invoked only through the NestJS CQRS command and
//! query bus. OrdersController declares no HTTP methods, so this module does
//! not publish a new checkout endpoint in PR3.
pub mod checkout;

use axum::{
    Json,
    http::{Method, StatusCode, Uri},
};
use sea_orm::{EntityTrait, PaginatorTrait, QuerySelect};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    common::{Pagination, date_time, paging, plain_not_found, success_verb},
    entity::order,
    error::{AppError, AppResult},
    state::AppState,
};

// Nest's OrdersController has @Controller('orders') but no method handlers.
// Axum's default 404 has an empty body, so preserve Nest's JSON envelope for
// the exact controller root instead of advertising a nonexistent checkout API.
pub async fn unavailable(method: Method, uri: Uri) -> (StatusCode, Json<Value>) {
    let path = uri.path();
    (
        StatusCode::NOT_FOUND,
        Json(json!({"message":format!("Cannot {method} {path}"),
        "error":"Not Found","statusCode":404})),
    )
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutInput {
    pub order: OrderInput,
    pub order_items: Vec<OrderItemInput>,
    pub payment: PaymentInput,
    pub shipment: ShipmentInput,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderInput {
    pub user_id: Uuid,
    pub address_id: Uuid,
    pub voucher_ids: Option<Vec<Uuid>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderItemInput {
    pub product_id: Uuid,
    pub discount_id: Option<Uuid>,
    pub quantity: i32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaymentInput {
    pub method: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ShipmentInput {
    pub courier: String,
    pub service: String,
    pub cost: f64,
    #[serde(deserialize_with = "crate::common::coerce_js_date")]
    pub estimated: String,
    #[serde(deserialize_with = "crate::common::required_nullable")]
    pub description: Option<String>,
}

impl CheckoutInput {
    pub fn validate(&self) -> AppResult<()> {
        if self.order_items.is_empty() {
            return Err(AppError::BadRequest(
                "orderItems must include at least one product".into(),
            ));
        }
        if self.order.voucher_ids.as_ref().is_some_and(|v| v.len() > 2) {
            return Err(AppError::BadRequest("Voucher ID max 2".into()));
        }
        for (field, text, max) in [
            ("courier", &self.shipment.courier, 50),
            ("service", &self.shipment.service, 50),
            ("method", &self.payment.method, 50),
        ] {
            if text.is_empty() || text.chars().count() > max {
                return Err(AppError::BadRequest(format!(
                    "{field} must be 1–{max} characters"
                )));
            }
        }
        if self
            .shipment
            .description
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.chars().count() > 255)
        {
            return Err(AppError::BadRequest(
                "description must be 1–255 characters".into(),
            ));
        }
        if !self.shipment.cost.is_finite() {
            return Err(AppError::BadRequest("cost must be a finite number".into()));
        }
        super::catalog_common::parse_datetime(&self.shipment.estimated)?;
        Ok(())
    }
}

pub fn body(m: order::Model) -> Value {
    json!({"id":m.id,"userId":m.user_id,"addressId":m.address_id,
        "voucherAmount":m.voucher_amount,"discountAmount":m.discount_amount,
        "totalPrice":m.total_price,"finalPrice":m.final_price,"status":m.status,
        "createdAt":date_time(m.created_at),"updatedAt":date_time(m.updated_at)})
}

pub async fn find_all(s: &AppState, query: Pagination) -> AppResult<Value> {
    let (page, limit) = query.checked()?;
    let total = order::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(plain_not_found("Orders"));
    }
    let rows = order::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(
        json!({"message":success_verb("retrieved","Orders"),"data":rows.into_iter().map(body).collect::<Vec<_>>(),"paging":paging(page,limit,total)}),
    )
}
