use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use super::{catalog_common::detail, products};
use crate::{
    common::{not_found, parse_uuid},
    entity::{category, order_item, product, product_information, review},
    error::{AppError, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/information/{id}", get(information))
        .route("/{id}", get(one))
}

async fn one(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let m = product::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Product"))?;
    let category = category::Entity::find_by_id(m.category_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?;
    let reviews = review::Entity::find()
        .filter(review::Column::ProductId.eq(m.id))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let rate = if reviews.is_empty() {
        0.0
    } else {
        reviews.iter().map(|r| f64::from(r.rating)).sum::<f64>() / reviews.len() as f64
    };
    let orders = order_item::Entity::find()
        .filter(order_item::Column::ProductId.eq(m.id))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let sales = orders.iter().map(|i| i.quantity as i64).sum::<i64>();
    let d = products::active_discount(&s, m.id).await?;
    let (discount_price, discount_type, discount_value) =
        products::price_with_discount(m.price, d.as_ref());
    Ok(Json(detail(
        "Product",
        json!({"id":m.id,"name":m.name,"imageUrl":products::current_image_url(m.image_url,m.id,&s.config.public_base_url).unwrap_or_default(),"description":m.description.unwrap_or_default(),"rate":rate,"category":category.map(|c|c.name).unwrap_or_default(),"sales":sales,"price":m.price,"discountPrice":discount_price,"discountType":discount_type,"discountValue":discount_value}),
    )))
}

fn render_information(
    description: &str,
    why: &str,
    highlights: &[String],
    serving: &[String],
    size: &str,
    storage: &str,
    shelf_life: &str,
) -> String {
    format!("{description}\n\n🍦 Highlights\n{}\n\n🍨 Serving Suggestions\n{}\n\n📦 Product Details\n• Size: {size}\n• Storage: {storage}\n• Shelf life: {shelf_life}\n\n⚡ Why Choose This Product?\n{why}",highlights.iter().map(|v|format!("• {v}")).collect::<Vec<_>>().join("\n"),serving.iter().map(|v|format!("• {v}")).collect::<Vec<_>>().join("\n")).trim().to_owned()
}

async fn information(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let m = product_information::Entity::find()
        .filter(product_information::Column::ProductId.eq(id))
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Product"))?;
    let p = product::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Product"))?;
    let highlights = serde_json::from_value::<Vec<String>>(m.highlights)
        .map_err(|_| AppError::Internal("Invalid product highlights data".into()))?;
    let serving = serde_json::from_value::<Vec<String>>(m.serving_suggestions)
        .map_err(|_| AppError::Internal("Invalid product servingSuggestions data".into()))?;
    let details = m
        .product_details
        .as_object()
        .ok_or_else(|| AppError::Internal("Invalid productDetails data".into()))?;
    let read = |name: &str| -> AppResult<&str> {
        details
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Internal(format!("Invalid productDetails.{name}")))
    };
    let result = render_information(
        p.description.as_deref().unwrap_or_default(),
        &m.why_choose,
        &highlights,
        &serving,
        read("size")?,
        read("storage")?,
        read("shelfLife")?,
    );
    Ok(Json(detail("Product", json!({"information":result}))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_information_sections_in_legacy_order() {
        let info = render_information(
            "Nice",
            "Because",
            &["Fresh".into()],
            &["Chill".into()],
            "1 L",
            "Freezer",
            "1 year",
        );
        assert!(info.starts_with("Nice\n\n🍦 Highlights"));
        assert!(info.contains("• Shelf life: 1 year"));
        assert!(info.ends_with("Because"));
    }
}
