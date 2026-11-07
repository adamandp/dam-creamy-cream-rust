use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Value, json};

use crate::{
    common::{date_time, paging, success_verb},
    error::{AppError, AppResult},
};

pub fn collection(name: &str, data: Vec<Value>, page: u64, limit: u64, total: u64) -> Value {
    json!({ "message": success_verb("retrieved", name), "data": data, "paging": paging(page, limit, total) })
}

pub fn detail(name: &str, data: Value) -> Value {
    json!({ "message": success_verb("retrieved", name), "data": data })
}

pub fn timestamps(created: NaiveDateTime, updated: NaiveDateTime) -> (String, String) {
    (date_time(created), date_time(updated))
}

pub fn parse_datetime(s: &str) -> AppResult<NaiveDateTime> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc).naive_utc())
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f"))
        .map_err(|_| AppError::BadRequest("Invalid date; ISO-8601 expected".into()))
}

pub fn require_text(value: &str, min: usize, max: usize, field: &str) -> AppResult<()> {
    if value.chars().count() < min || value.chars().count() > max {
        Err(AppError::BadRequest(format!(
            "{field} must be {min}–{max} characters"
        )))
    } else {
        Ok(())
    }
}

pub fn require_positive(value: f64, min: f64, max: f64, field: &str) -> AppResult<()> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        Err(AppError::BadRequest(format!(
            "{field} must be between {min} and {max}"
        )))
    } else {
        Ok(())
    }
}

pub fn reject_no_change(changed: &[Option<bool>]) -> AppResult<()> {
    if changed.iter().all(Option::is_none) || changed.contains(&Some(true)) {
        return Err(AppError::BadRequest(
            "📭 No changes detected. Everything is already up to date! 👍".into(),
        ));
    }
    Ok(())
}
