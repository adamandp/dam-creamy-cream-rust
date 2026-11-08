use std::sync::OnceLock;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, PaginatorTrait, QuerySelect};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    authorization,
    catalog_common::{collection, detail},
};
use crate::{
    common::{Pagination, date_time, not_found, parse_uuid, plain_not_found, success},
    entity::shipment::{self, ShipmentStatus},
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    status: ShipmentStatus,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostInput {
    #[serde(deserialize_with = "crate::common::coerce_js_number")]
    origin: f64,
    #[serde(deserialize_with = "crate::common::coerce_js_number")]
    destination: f64,
    #[serde(deserialize_with = "crate::common::coerce_js_number")]
    weight: f64,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list))
        .route("/cost", post(cost))
        .route("/{id}", get(one))
        .route("/{id}/status", axum::routing::patch(update_status))
}

fn body(m: shipment::Model) -> Value {
    json!({"id":m.id,"orderId":m.order_id,"courier":m.courier,"service":m.service,
        "cost":m.cost,"estimated":date_time(m.estimated),
        "deliveredAt":m.delivered_at.map(date_time),"description":m.description,
        "trackingNumber":m.tracking_number,"status":m.status,
        "createdAt":date_time(m.created_at),"updatedAt":date_time(m.updated_at)})
}

async fn list(
    State(s): State<AppState>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = q.checked()?;
    let total = shipment::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(plain_not_found("Shipment"));
    }
    let records = shipment::Entity::find()
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Shipment",
        records.into_iter().map(body).collect(),
        page,
        limit,
        total,
    )))
}

async fn one(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let m = shipment::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Shipment"))?;
    Ok(Json(detail("Shipment", body(m))))
}

async fn update_status(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<UpdateStatus>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let m = shipment::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Shipment"))?;
    let mut row: shipment::ActiveModel = m.into();
    row.status = Set(req.status);
    row.updated_at = Set(Utc::now().naive_utc());
    row.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Shipment")))
}

const COURIERS: &str =
    "jne:sicepat:ide:sap:jnt:ninja:tiki:lion:anteraja:pos:ncs:rex:rpx:sentral:star:wahana:dse";

// Single client, not one TCP/TLS pool per shipping-price request.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("valid HTTP client")
    })
}

async fn fetch_quote(base_url: &str, api_key: &str, req: &CostInput) -> AppResult<Value> {
    let failure = || {
        AppError::ExternalService(
            "❌ Oops! Failed to create Shipping cost. Try again later! 🤦‍♂️".into(),
        )
    };
    let response = client()
        .post(format!(
            "{}/calculate/domestic-cost",
            base_url.trim_end_matches('/')
        ))
        .header("key", api_key)
        .form(&[
            ("origin", req.origin.to_string()),
            ("destination", req.destination.to_string()),
            ("weight", req.weight.to_string()),
            ("courier", COURIERS.to_owned()),
        ])
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error=%e,"RajaOngkir request failed");
            failure()
        })?;
    let response = response.error_for_status().map_err(|e| {
        tracing::error!(error=%e,"RajaOngkir rejected request");
        failure()
    })?;
    let payload: Value = response.json().await.map_err(|e| {
        tracing::error!(error=%e,"Invalid RajaOngkir response");
        failure()
    })?;
    let meta = payload.get("meta").ok_or_else(failure)?;
    let message = meta.get("message").ok_or_else(failure)?;
    let status = meta.get("code").ok_or_else(failure)?;
    let data = payload.get("data").ok_or_else(failure)?;
    Ok(json!({"message":message,"status":status,"data":data}))
}

async fn cost(ApiJson(req): ApiJson<CostInput>) -> AppResult<(StatusCode, Json<Value>)> {
    let failure = || {
        AppError::ExternalService(
            "❌ Oops! Failed to create Shipping cost. Try again later! 🤦‍♂️".into(),
        )
    };
    let url = std::env::var("RAJAONGKIR_BASE_URL").map_err(|_| failure())?;
    let key = std::env::var("RAJAONGKIR_API_KEY").map_err(|_| failure())?;
    Ok((
        StatusCode::CREATED,
        Json(fetch_quote(&url, &key, &req).await?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Form, http::HeaderMap};
    use std::collections::HashMap;

    #[test]
    fn cost_dto_matches_zod_coercion_and_strictness() {
        let dto: CostInput =
            serde_json::from_value(json!({"origin":"1","destination":20,"weight":true})).unwrap();
        assert_eq!((dto.origin, dto.destination, dto.weight), (1.0, 20.0, 1.0));
        assert!(
            serde_json::from_value::<CostInput>(
                json!({"origin":1,"destination":2,"weight":100,"extra":5})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn quote_sends_form_data_and_maps_success_response() {
        async fn mocked_provider(
            headers: HeaderMap,
            Form(fields): Form<HashMap<String, String>>,
        ) -> Json<Value> {
            assert_eq!(headers["key"], "test-rajaongkir-key");
            assert_eq!(fields["origin"], "12");
            assert_eq!(fields["destination"], "34");
            assert_eq!(fields["weight"], "1000");
            assert_eq!(fields["courier"], COURIERS);
            Json(json!({"meta":{"message":"success","code":200},"data":[{"service":"REG"}]}))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/calculate/domestic-cost", post(mocked_provider)),
            )
            .await
            .unwrap();
        });
        let input = CostInput {
            origin: 12.0,
            destination: 34.0,
            weight: 1000.0,
        };
        let result = fetch_quote(&base_url, "test-rajaongkir-key", &input)
            .await
            .unwrap();
        assert_eq!(
            result,
            json!({"message":"success","status":200,"data":[{"service":"REG"}]})
        );
        server.abort();
    }
}
