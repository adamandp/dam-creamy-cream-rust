//! Cart endpoints. The NestJS cart belongs to the authenticated user and
//! stores quantities independently of available inventory.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{authorization, products};
use crate::{
    common::{Pagination, not_found, paging, success},
    entity::{cart, cart_item, category, discount, product, product_discount},
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CartInput {
    pub product_id: Uuid,
    pub quantity: i32,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/user/add", post(add))
        .route("/user/remove", post(remove))
        .route("/user", get(by_user))
}

fn validate_quantity(quantity: i32) -> AppResult<()> {
    if quantity < 1 {
        return Err(AppError::Validation(vec![json!({
            "origin":"number","code":"too_small","minimum":1,"inclusive":true,
            "path":["quantity"],
            "message":"🔡 Quantity must be at least 1 characters. Keep going! 💪 {Minimum Length}"
        })]));
    }
    if quantity > 100 {
        return Err(AppError::Validation(vec![json!({
            "origin":"number","code":"too_big","maximum":100,"inclusive":true,
            "path":["quantity"],
            "message":"🚀 Whoa! Quantity is too long. Maximum allowed is 100 characters! ✍️ {Maximum Length}"
        })]));
    }
    Ok(())
}

async fn add(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<CartInput>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let actor = authorization::authenticate(&s, &headers).await?;
    validate_quantity(input.quantity)?;
    if product::Entity::find_by_id(input.product_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Product"));
    }
    let tx = s.db.begin().await.map_err(AppError::db)?;
    let cart = cart::Entity::find()
        .filter(cart::Column::UserId.eq(actor.id))
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Cart"))?;
    let existing = cart_item::Entity::find()
        .filter(cart_item::Column::CartId.eq(cart.id))
        .filter(cart_item::Column::ProductId.eq(input.product_id))
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?;
    let now = Utc::now().naive_utc();
    if let Some(item) = existing {
        let quantity = item
            .quantity
            .checked_add(input.quantity)
            .ok_or_else(|| AppError::Conflict("Cart quantity overflow".into()))?;
        let mut row: cart_item::ActiveModel = item.into();
        row.quantity = Set(quantity);
        row.updated_at = Set(now);
        row.update(&tx).await.map_err(AppError::db)?;
    } else {
        cart_item::ActiveModel {
            id: Set(Uuid::new_v4()),
            cart_id: Set(cart.id),
            product_id: Set(input.product_id),
            quantity: Set(input.quantity),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&tx)
        .await
        .map_err(AppError::db)?;
    }
    tx.commit().await.map_err(AppError::db)?;
    Ok((StatusCode::CREATED, Json(success("created", "Cart"))))
}

async fn remove(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<CartInput>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let actor = authorization::authenticate(&s, &headers).await?;
    validate_quantity(input.quantity)?;
    if product::Entity::find_by_id(input.product_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Product"));
    }
    let tx = s.db.begin().await.map_err(AppError::db)?;
    let cart = cart::Entity::find()
        .filter(cart::Column::UserId.eq(actor.id))
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Cart"))?;
    let item = cart_item::Entity::find()
        .filter(cart_item::Column::CartId.eq(cart.id))
        .filter(cart_item::Column::ProductId.eq(input.product_id))
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Cart Item"))?;
    if item.quantity <= input.quantity {
        cart_item::Entity::delete_by_id(item.id)
            .exec(&tx)
            .await
            .map_err(AppError::db)?;
    } else {
        let mut row: cart_item::ActiveModel = item.clone().into();
        row.quantity = Set(item.quantity - input.quantity);
        row.updated_at = Set(Utc::now().naive_utc());
        row.update(&tx).await.map_err(AppError::db)?;
    }
    tx.commit().await.map_err(AppError::db)?;
    // NestJS @Post without @HttpCode returns 201 even on a delete-like action.
    Ok((StatusCode::CREATED, Json(success("deleted", "Cart"))))
}

async fn by_user(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let actor = authorization::authenticate(&s, &headers).await?;
    let (page, limit) = q.checked()?;
    let user_cart = cart::Entity::find()
        .filter(cart::Column::UserId.eq(actor.id))
        .one(&s.db)
        .await
        .map_err(AppError::db)?;
    let Some(user_cart) = user_cart else {
        return Ok(Json(
            json!({"message":crate::common::success_verb("retrieved", "Cart"),"data":[],"paging":paging(page,limit,0)}),
        ));
    };
    let query = cart_item::Entity::find().filter(cart_item::Column::CartId.eq(user_cart.id));
    let total = query.clone().count(&s.db).await.map_err(AppError::db)?;
    if total == 0 {
        return Ok(Json(
            json!({"message":crate::common::success_verb("retrieved", "Cart"),"data":[],"paging":paging(page,limit,0)}),
        ));
    }
    let items = query
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let mut data = Vec::with_capacity(items.len());
    let now = Utc::now().naive_utc();
    for item in items {
        let p = product::Entity::find_by_id(item.product_id)
            .one(&s.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| not_found("Product"))?;
        let c = category::Entity::find_by_id(p.category_id)
            .one(&s.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| not_found("Category"))?;
        // Legacy cart response reads productDiscounts, even when the link is inactive.
        let relation = product_discount::Entity::find()
            .filter(product_discount::Column::ProductId.eq(p.id))
            .one(&s.db)
            .await
            .map_err(AppError::db)?;
        let d = if let Some(link) = relation {
            discount::Entity::find_by_id(link.discount_id)
                .one(&s.db)
                .await
                .map_err(AppError::db)?
        } else {
            None
        };
        let discount_price = d
            .filter(|d| d.value.is_some() && d.start_date <= now && d.end_date >= now)
            .map(|d| match d.discount_type {
                discount::DiscountType::Percentage => {
                    p.price - p.price * d.value.unwrap_or_default() / 100.0
                }
                _ => (p.price - d.value.unwrap_or_default()).max(0.0),
            });
        data.push(json!({"id":p.id,"name":p.name,
            "imageUrl": products::current_image_url(p.image_url,p.id,&s.config.public_base_url),
            "category":c.name,"price":p.price,"discountPrice":discount_price,"qty":item.quantity}));
    }
    Ok(Json(
        json!({"message":crate::common::success_verb("retrieved", "User Cart"),"data":data,"paging":paging(page,limit,total)}),
    ))
}
