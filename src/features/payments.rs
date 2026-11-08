//! The NestJS PaymentsController is still a generated scaffold. Preserve its
//! literal responses; do not invent payment operations before PR4/5.
use crate::state::AppState;
use axum::{Router, http::StatusCode, routing::get};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(|| async { "This action returns all payments" })
                .post(|| async { (StatusCode::CREATED, "This action adds a new payment") }),
        )
        .route("/{id}", get(one).patch(update).delete(remove))
}

fn js_number(s: &str) -> String {
    let text = s.trim();
    if text.is_empty() {
        return "0".into();
    }
    let radix = [
        ("0x", 16),
        ("0X", 16),
        ("0b", 2),
        ("0B", 2),
        ("0o", 8),
        ("0O", 8),
    ];
    for (prefix, base) in radix {
        if let Some(digits) = text.strip_prefix(prefix) {
            return u64::from_str_radix(digits, base)
                .map_or_else(|_| "NaN".into(), |n| (n as f64).to_string());
        }
    }
    match text.parse::<f64>() {
        Ok(n) if n.is_nan() => "NaN".into(),
        Ok(n) if n.is_infinite() => {
            let sign = if n.is_sign_negative() { "-" } else { "" };
            format!("{sign}Infinity")
        }
        Ok(n) if n == 0.0 => "0".into(),
        Ok(n) => n.to_string(),
        Err(_) => "NaN".into(),
    }
}

async fn one(axum::extract::Path(id): axum::extract::Path<String>) -> String {
    format!("This action returns a #{} payment", js_number(&id))
}
async fn update(axum::extract::Path(id): axum::extract::Path<String>) -> String {
    format!("This action updates a #{} payment", js_number(&id))
}
async fn remove(axum::extract::Path(id): axum::extract::Path<String>) -> String {
    format!("This action removes a #{} payment", js_number(&id))
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_controller_uses_js_numeric_coercion() {
        assert_eq!(super::js_number("5"), "5");
        assert_eq!(super::js_number("missing"), "NaN");
        assert_eq!(super::js_number("0x10"), "16");
        assert_eq!(super::js_number("0b101"), "5");
        assert_eq!(super::js_number("Infinity"), "Infinity");
        assert_eq!(super::js_number("-0"), "0");
    }
}
