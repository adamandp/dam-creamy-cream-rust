//! The NestJS PaymentsController is still a generated scaffold. Preserve its
//! literal responses; do not invent payment operations before PR4/5.
use axum::{Router, routing::get};
use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(|| async {"This action returns all payments"}).post(|| async {"This action adds a new payment"}))
        .route("/{id}", get(one).patch(update).delete(remove))
}

fn js_number(s: &str) -> String {
    if s.trim().is_empty() { return "0".into(); }
    match s.parse::<f64>() {
        Ok(n) if n.is_finite() => n.to_string(),
        _ => "NaN".into(),
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
    }
}
