use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use std::{cmp::Ordering, collections::HashMap};
use uuid::Uuid;

use super::catalog_common::detail;
use crate::{
    common::{not_found, parse_uuid},
    entity::{
        category,
        discount::{self, DiscountType},
        order_item, product, product_discount, review,
    },
    error::{AppError, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/classic", get(classic))
        .route("/categories", get(categories))
        .route("/offers", get(offers))
        .route("/related/{id}", get(related))
}

struct Ranked {
    model: product::Model,
    rating: f64,
    sales: usize,
    discount_type: Option<DiscountType>,
    discount_value: Option<f64>,
    discount_amount: f64,
}
impl Ranked {
    fn json(&self) -> Value {
        let discount_price = self
            .discount_value
            .filter(|v| *v != 0.0)
            .map(|_| self.model.price - self.discount_amount);
        json!({"id":self.model.id,"name":self.model.name,"imageUrl":self.model.image_url.as_deref().unwrap_or(""),
            "description":self.model.description.as_deref().unwrap_or(""),"rate":self.rating,"price":self.model.price,
            "discountPrice":discount_price,"discountType":self.discount_type,"discountValue":self.discount_value})
    }
}
fn by_rating_sales(a: &Ranked, b: &Ranked) -> Ordering {
    b.rating
        .total_cmp(&a.rating)
        .then_with(|| b.sales.cmp(&a.sales))
}
async fn ranked(s: &AppState, products: Vec<product::Model>) -> AppResult<Vec<Ranked>> {
    if products.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = products.iter().map(|p| p.id).collect();
    let reviews = review::Entity::find()
        .filter(review::Column::ProductId.is_in(ids.clone()))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let orders = order_item::Entity::find()
        .filter(order_item::Column::ProductId.is_in(ids.clone()))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let links = product_discount::Entity::find()
        .filter(product_discount::Column::ProductId.is_in(ids))
        .filter(product_discount::Column::IsActive.eq(true))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let discount_ids: Vec<Uuid> = links.iter().map(|p| p.discount_id).collect();
    let discounts = if discount_ids.is_empty() {
        Vec::new()
    } else {
        discount::Entity::find()
            .filter(discount::Column::Id.is_in(discount_ids))
            .all(&s.db)
            .await
            .map_err(AppError::db)?
    };
    let discount_map: HashMap<_, _> = discounts.into_iter().map(|d| (d.id, d)).collect();
    let link_map: HashMap<_, _> = links
        .into_iter()
        .map(|l| (l.product_id, l.discount_id))
        .collect();
    let mut rating_map: HashMap<Uuid, (f64, usize)> = HashMap::new();
    for r in reviews {
        let item = rating_map.entry(r.product_id).or_default();
        item.0 += f64::from(r.rating);
        item.1 += 1;
    }
    let mut sales_map: HashMap<Uuid, usize> = HashMap::new();
    for o in orders {
        *sales_map.entry(o.product_id).or_default() += 1;
    }
    Ok(products
        .into_iter()
        .map(|p| {
            let (sum, count) = rating_map.get(&p.id).copied().unwrap_or_default();
            let d = link_map.get(&p.id).and_then(|id| discount_map.get(id));
            let discount_amount = d
                .map(|d| match &d.discount_type {
                    DiscountType::Fixed => d.value.unwrap_or(0.0),
                    DiscountType::Percentage => d.value.unwrap_or(0.0) * p.price / 100.0,
                    _ => 0.0,
                })
                .unwrap_or(0.0);
            Ranked {
                rating: if count > 0 { sum / count as f64 } else { 0.0 },
                sales: *sales_map.get(&p.id).unwrap_or(&0),
                discount_type: d.map(|d| d.discount_type.clone()),
                discount_value: d.and_then(|d| d.value),
                discount_amount,
                model: p,
            }
        })
        .collect())
}
async fn classic(State(s): State<AppState>) -> AppResult<Json<Value>> {
    let cats = category::Entity::find()
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let ids: Vec<Uuid> = cats
        .into_iter()
        .filter(|c| c.name.to_lowercase().contains("classic"))
        .map(|c| c.id)
        .collect();
    let products = if ids.is_empty() {
        Vec::new()
    } else {
        product::Entity::find()
            .filter(product::Column::IsActive.eq(true))
            .filter(product::Column::CategoryId.is_in(ids))
            .all(&s.db)
            .await
            .map_err(AppError::db)?
    };
    if products.is_empty() {
        return Err(not_found("Classic Recommendations"));
    }
    let mut items = ranked(&s, products).await?;
    items.sort_by(by_rating_sales);
    Ok(Json(detail(
        "Classic Recommendations",
        json!(items.iter().map(Ranked::json).collect::<Vec<_>>()),
    )))
}
async fn categories(State(s): State<AppState>) -> AppResult<Json<Value>> {
    let cats = category::Entity::find()
        .filter(category::Column::IsActive.eq(true))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    if cats.is_empty() {
        return Err(not_found("Categories Recommendations"));
    }
    let mut data = Vec::new();
    for c in cats {
        let products = product::Entity::find()
            .filter(product::Column::CategoryId.eq(c.id))
            .filter(product::Column::IsActive.eq(true))
            .all(&s.db)
            .await
            .map_err(AppError::db)?;
        let mut ranked = ranked(&s, products).await?;
        ranked.sort_by_key(|a| std::cmp::Reverse(a.sales));
        if let Some(p) = ranked.first() {
            data.push(json!({"id":p.model.id,"imageUrl":p.model.image_url.as_deref().unwrap_or(""),"category":c.name,"categoryId":c.id}));
        }
    }
    Ok(Json(detail("Categories Recommendations", json!(data))))
}
async fn offers(State(s): State<AppState>) -> AppResult<Json<Value>> {
    let links = product_discount::Entity::find()
        .filter(product_discount::Column::IsActive.eq(true))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let ids: Vec<Uuid> = links.into_iter().map(|l| l.product_id).collect();
    let products = if ids.is_empty() {
        Vec::new()
    } else {
        product::Entity::find()
            .filter(product::Column::IsActive.eq(true))
            .filter(product::Column::Id.is_in(ids))
            .all(&s.db)
            .await
            .map_err(AppError::db)?
    };
    if products.is_empty() {
        return Err(not_found("Offer Recommendations"));
    }
    // Prisma's take(5) applies before the subsequent discount/rating sort.
    let mut items = ranked(&s, products.into_iter().take(5).collect()).await?;
    items.sort_by(|a, b| {
        b.discount_amount
            .total_cmp(&a.discount_amount)
            .then_with(|| by_rating_sales(a, b))
    });
    Ok(Json(detail(
        "Offer Recommendations",
        json!(items.iter().map(Ranked::json).collect::<Vec<_>>()),
    )))
}
async fn related(State(s): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let id = parse_uuid(&id)?;
    let target = product::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Related Recommendations"))?;
    let products = product::Entity::find()
        .filter(product::Column::IsActive.eq(true))
        .filter(product::Column::Id.ne(id))
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let mut primary = Vec::new();
    let mut extras = Vec::new();
    for p in products {
        if p.category_id == target.category_id {
            primary.push(p);
        } else {
            extras.push(p);
        }
    }
    let mut items = ranked(&s, primary).await?;
    items.sort_by(by_rating_sales);
    if items.len() < 5 {
        let mut rest = ranked(&s, extras).await?;
        items.append(&mut rest);
        items.sort_by(by_rating_sales);
    }
    items.truncate(5);
    if items.is_empty() {
        return Err(not_found("Related Recommendations"));
    }
    Ok(Json(detail(
        "Related Recommendations",
        json!(items.iter().map(Ranked::json).collect::<Vec<_>>()),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn product(name: &str, rating: f64, sales: usize) -> Ranked {
        let now = Utc::now().naive_utc();
        Ranked {
            model: product::Model {
                id: Uuid::new_v4(),
                category_id: Uuid::new_v4(),
                discount_id: None,
                name: name.into(),
                image_url: None,
                description: None,
                price: 100.0,
                is_active: true,
                created_at: now,
                updated_at: now,
            },
            rating,
            sales,
            discount_type: Some(DiscountType::Percentage),
            discount_value: Some(20.0),
            discount_amount: 20.0,
        }
    }

    #[test]
    fn ranking_prioritizes_average_rating_then_sales() {
        let mut list = [
            product("high-sales", 4.0, 100),
            product("top-rated", 5.0, 1),
            product("tie", 5.0, 10),
        ];
        list.sort_by(by_rating_sales);
        assert_eq!(
            list.iter()
                .map(|r| r.model.name.as_str())
                .collect::<Vec<_>>(),
            vec!["tie", "top-rated", "high-sales"]
        );
    }

    #[test]
    fn recommendation_response_retains_nullability_and_discount_math() {
        let row = product("Classic", 0.0, 0);
        let value = row.json();
        assert_eq!(value["discountPrice"], 80.0);
        assert_eq!(value["discountValue"], 20.0);
        assert_eq!(value["imageUrl"], "");
        assert_eq!(value["rate"], 0.0);
    }
}
