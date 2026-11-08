use chrono::{DateTime, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use serde::{Deserialize, de::Error as _};
use serde_json::{Value, json};

use crate::error::{AppError, AppResult};

// Zod's `.nullable()` requires that a key exists but accepts an explicit
// `null`. Serde's ordinary Option<T> would also accept a *missing* key.
pub fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

// This intentionally mirrors z.coerce.boolean() / JavaScript Boolean(value),
// including the surprising fact that the nonempty string "false" is true.
fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

pub fn coerce_js_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(js_truthy(&value))
}

pub fn coerce_js_bool_optional<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    coerce_js_bool(deserializer).map(Some)
}

// Zod's z.coerce.date() runs JavaScript Date(value): unlike parsing a string,
// numbers are milliseconds since the Unix epoch and null/boolean are also
// accepted. Keep the DTO representation as an ISO string for parse_datetime.
pub fn coerce_js_date<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    let millis = match value {
        Value::String(text) => return Ok(text),
        Value::Null => 0,
        Value::Bool(value) => i64::from(value),
        Value::Number(number) => {
            let numeric = number
                .as_f64()
                .ok_or_else(|| D::Error::custom("Invalid date"))?;
            if !numeric.is_finite() || numeric.abs() > 8.64e15 {
                return Err(D::Error::custom("Invalid date"));
            }
            numeric as i64
        }
        _ => return Err(D::Error::custom("Invalid date")),
    };
    DateTime::<Utc>::from_timestamp_millis(millis)
        .map(|date| date.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| D::Error::custom("Invalid date"))
}

pub fn coerce_js_date_optional<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    coerce_js_date(deserializer).map(Some)
}

#[derive(Deserialize, Default)]
pub struct Pagination {
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

impl Pagination {
    pub fn checked(&self) -> AppResult<(u64, u64)> {
        let page = self.page.unwrap_or(1);
        let limit = self.limit.unwrap_or(10);
        if !(1..=100).contains(&page) || !(1..=100).contains(&limit) {
            return Err(AppError::BadRequest(
                "page and limit must be between 1 and 100".into(),
            ));
        }
        Ok((page as u64, limit as u64))
    }
}

pub fn paging(page: u64, limit: u64, total: u64) -> Value {
    json!({
        "currentPage": page,
        "pageSize": limit,
        "totalItems": total,
        "totalPages": total.div_ceil(limit),
    })
}

pub fn date_time(value: NaiveDateTime) -> String {
    DateTime::<Utc>::from_naive_utc_and_offset(value, Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn date(value: NaiveDate) -> String {
    format!("{}T00:00:00.000Z", value.format("%Y-%m-%d"))
}

pub fn success_verb(verb: &str, resource: &str) -> String {
    match verb {
        "created" => format!("🎉 {resource} created successfully! Welcome aboard! 🚀"),
        "retrieved" => format!("📦 {resource} retrieved successfully! Smooth sailing! 😎"),
        "updated" => format!("🔄 {resource} updated successfully! Looking fresh! ✨"),
        "deleted" => format!("🗑️ {resource} deleted successfully! Sayonara! 👋"),
        _ => resource.to_owned(),
    }
}

pub fn success(verb: &str, resource: &str) -> Value {
    json!({"message": success_verb(verb, resource)})
}

pub fn parse_uuid(value: &str) -> AppResult<uuid::Uuid> {
    uuid::Uuid::parse_str(value)
        .map_err(|_| AppError::BadRequest("Validation failed (uuid is expected)".into()))
}

// Preserve the legacy NestJS validateUpdate contract. A PATCH with no fields
// or with even one supplied field identical to its persisted value is a 400.
// An entry of None means the field was omitted; Some(true) means unchanged.
pub fn require_changed_update(fields: &[(&str, Option<bool>)]) -> AppResult<()> {
    if fields.iter().all(|(_, same)| same.is_none()) {
        return Err(AppError::BadRequest(
            "📭 No changes detected. Everything is already up to date! 👍".into(),
        ));
    }
    let unchanged = fields
        .iter()
        .filter_map(|(name, same)| (*same == Some(true)).then_some(*name))
        .collect::<Vec<_>>();
    if !unchanged.is_empty() {
        return Err(AppError::BadRequest(format!(
            "📭 No changes detected in {} fields. Everything’s already up to date!",
            unchanged.join(", ")
        )));
    }
    Ok(())
}

pub fn not_found(resource: &str) -> AppError {
    AppError::NotFound(success_not_found_message(resource))
}

pub fn plain_not_found(resource: &str) -> AppError {
    AppError::PlainNotFound(success_not_found_message(resource))
}

pub fn success_not_found_message(resource: &str) -> String {
    format!("🔍 {resource} not found. Maybe it teleported to another dimension? 🛸")
}

#[cfg(test)]
mod update_tests {
    use super::*;

    #[test]
    fn patch_requires_a_changed_field_and_rejects_mixed_unchanged_values() {
        assert!(require_changed_update(&[]).is_err());
        assert!(require_changed_update(&[("city", Some(true))]).is_err());
        assert!(require_changed_update(&[("city", None)]).is_err());
        assert!(require_changed_update(&[("city", Some(false))]).is_ok());
        assert!(
            require_changed_update(&[("city", Some(false)), ("province", Some(true))]).is_err()
        );
    }
}
