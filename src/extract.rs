use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Query, Request},
    http::request::Parts,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::AppError;

/// Translate Axum extractor failures into the API's JSON error envelope.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(req, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|err| {
                let message = err.body_text();
                legacy_json_validation::<T>(&message).unwrap_or(AppError::BadRequest(message))
            })
    }
}

pub struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        if let Some(errors) = legacy_query_validation::<T>(parts.uri.query().unwrap_or("")) {
            return Err(AppError::Validation(errors));
        }
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|err| AppError::BadRequest(err.body_text()))
    }
}

// NestJS uses ZodValidationPipe for DTOs. Its public 400 envelope includes
// typed issues and has no `error` field, unlike Nest's HttpException errors.
// Preserve that shape for the common query DTOs and required create fields.
fn legacy_json_validation<T>(message: &str) -> Option<AppError> {
    let class_name = std::any::type_name::<T>();
    let is_category = class_name.ends_with("::categories::Create");
    let is_discount = class_name.ends_with("::discounts::Create");
    let is_voucher = class_name.ends_with("::vouchers::Create");
    let is_product = class_name.ends_with("::products::Create");
    let is_address = class_name.ends_with("::addresses::CreateAddressBody");
    if !(is_category
        || is_discount
        || is_voucher
        || is_product
        || class_name.ends_with("reviews::Create")
        || class_name.ends_with("product_discounts::Create")
        || is_address)
    {
        return None;
    }
    if (is_product || is_address)
        && let Some(field) = message
            .split("unknown field `")
            .nth(1)
            .and_then(|rest| rest.split('`').next())
    {
        return Some(AppError::Validation(vec![json!({
            "code":"unrecognized_keys","keys":[field],"path":[],
            "message":format!("Unrecognized key: \"{field}\"")
        })]));
    }
    if message.contains("isActive: invalid type:") && message.contains("expected a boolean") {
        return Some(AppError::Validation(vec![json!({
            "expected":"boolean","code":"invalid_type","path":["isActive"],
            "message":"⚖️ Is Active must be either true or false. No Schrödinger values! 🐱 {Invalid Boolean}"
        })]));
    }
    let field = message.split("missing field `").nth(1)?.split('`').next()?;
    let (expected, display) = match field {
        "name" if is_category => ("string", "Category Name"),
        "name" if is_discount => ("string", "Discount Name"),
        "name" => ("string", "Name"),
        "description" => ("string", "Description"),
        "isActive" => ("boolean", "Is Active"),
        "price" => ("number", "price"),
        "value" => ("number", "Value"),
        "quantity" => ("number", "Quantity"),
        "rating" => ("number", "Rating"),
        "productId" | "categoryId" | "discountId" | "userId" | "voucherId" => ("string", ""),
        _ => return None,
    };
    let message = match (expected, display) {
        ("string", "") => "Invalid input: expected string, received undefined".to_owned(),
        ("string", _) => {
            format!("📜 {display} must be a valid text. No weird symbols! 🚫 {{Invalid String}}")
        }
        ("number", _) => format!(
            "🔢 {display} must be a valid number. No funky math here! 🚫 {{Invalid Number}}"
        ),
        _ => format!(
            "⚖️ {display} must be either true or false. No Schrödinger values! 🐱 {{Invalid Boolean}}"
        ),
    };
    Some(AppError::Validation(vec![json!({
        "expected":expected,"code":"invalid_type","path":[field],"message":message
    })]))
}

fn legacy_query_validation<T>(raw: &str) -> Option<Vec<Value>> {
    let class_name = std::any::type_name::<T>();
    let catalog = class_name.ends_with("products::CatalogQuery");
    if !catalog && !class_name.ends_with("common::Pagination") {
        return None;
    }
    let params: Vec<(&str, &str)> = raw
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| part.split_once('=').unwrap_or((part, "")))
        .collect();
    let value = |field: &str| {
        params
            .iter()
            .rev()
            .find(|(k, _)| *k == field)
            .map(|(_, v)| *v)
    };
    let mut errors = Vec::new();

    if catalog {
        for (field, display) in [("q", "q"), ("cat", "Category")] {
            if let Some(v) = value(field) {
                if v.is_empty() {
                    errors.push(json!({"origin":"string","code":"too_small","minimum":1,"inclusive":true,"path":[field],"message":format!("🔡 {display} must be at least 1 characters. Keep going! 💪 {{Minimum Length}}") }));
                } else if v.len() > 100 {
                    errors.push(json!({"origin":"string","code":"too_big","maximum":100,"inclusive":true,"path":[field],"message":format!("🚀 Whoa! {display} is too long. Maximum allowed is 100 characters! ✍️ {{Maximum Length}}") }));
                }
            }
        }
        if let Some(sort) = value("sort")
            && !matches!(sort, "asc" | "desc" | "default")
        {
            errors.push(json!({"code":"invalid_value","values":["asc","desc","default"],"path":["sort"],"message":"🎭 Sort must be one of the following enum. Choose wisely! 🎯 {Invalid Enum}"}));
        }
        for field in ["min", "max"] {
            validate_query_number(&mut errors, field, value(field), 0.0, 1_000_000.0);
        }
    }
    for field in ["page", "limit"] {
        validate_query_number(&mut errors, field, value(field), 1.0, 100.0);
    }
    if catalog {
        let unknown: Vec<&str> = params
            .iter()
            .map(|(key, _)| *key)
            .filter(|key| {
                !matches!(
                    *key,
                    "q" | "sort" | "cat" | "min" | "max" | "page" | "limit"
                )
            })
            .collect();
        if !unknown.is_empty() {
            let message = if unknown.len() == 1 {
                format!("Unrecognized key: \"{}\"", unknown[0])
            } else {
                format!(
                    "Unrecognized keys: {}",
                    unknown
                        .iter()
                        .map(|key| format!("\"{key}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            errors.push(
                json!({"code":"unrecognized_keys","keys":unknown,"path":[],"message":message}),
            );
        }
    }
    (!errors.is_empty()).then_some(errors)
}

fn validate_query_number(
    errors: &mut Vec<Value>,
    field: &str,
    raw: Option<&str>,
    min: f64,
    max: f64,
) {
    let Some(raw) = raw else { return };
    let display = match field {
        "page" => "Page",
        "limit" => "Limit",
        _ => field,
    };
    let number = if raw.is_empty() {
        Some(0.0)
    } else {
        raw.parse::<f64>().ok()
    };
    let Some(number) = number.filter(|v| v.is_finite()) else {
        errors.push(json!({"expected":"number","code":"invalid_type","received":"NaN","path":[field],"message":format!("🔢 {display} must be a valid number. No funky math here! 🚫 {{Invalid Number}}") }));
        return;
    };
    if number < min {
        errors.push(json!({"origin":"number","code":"too_small","minimum":min,"inclusive":true,"path":[field],"message":format!("🔡 {display} must be at least {} characters. Keep going! 💪 {{Minimum Length}}", min as i64)}));
    } else if number > max {
        errors.push(json!({"origin":"number","code":"too_big","maximum":max,"inclusive":true,"path":[field],"message":format!("🚀 Whoa! {display} is too long. Maximum allowed is {} characters! ✍️ {{Maximum Length}}", max as i64)}));
    }
}
