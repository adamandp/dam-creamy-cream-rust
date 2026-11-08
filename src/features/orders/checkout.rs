//! Port of `orders/cqrs/commands/checkout/checkout.handler.ts` and its steps.
//! No route is exposed until the source Nest controller exposes one.
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, TransactionTrait,
};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

use super::{CheckoutInput, OrderItemInput};
use crate::{
    common::{not_found, success},
    entity::{
        address, cart, cart_item,
        discount::{self, DiscountType},
        inventory, order, order_item, order_voucher, payment, product, product_discount, shipment,
        user,
        user_voucher::{self, UserVoucherStatus},
        voucher::{self, VoucherGroup, VoucherType},
    },
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Clone, Debug)]
struct MappedItem {
    product_id: Uuid,
    price: f64,
    quantity: i32,
    discount_id: Option<Uuid>,
    discount_amount: f64,
    final_price: f64,
}

// This mirrors the original TS switch: FREE_ITEM has no numeric discount,
// and the discount is attached through discount.product_id, not by item ID.
fn map_items(
    products: &[product::Model],
    discounts: &[discount::Model],
    items: &[OrderItemInput],
) -> Vec<MappedItem> {
    products
        .iter()
        .filter_map(|p| {
            let original = items.iter().find(|i| i.product_id == p.id)?;
            let d = discounts.iter().find(|d| d.product_id == Some(p.id));
            let mut discount_amount = 0.0;
            if let Some(d) = d
                && let Some(value) = d.value.filter(|v| *v != 0.0)
            {
                discount_amount = match d.discount_type {
                    DiscountType::Fixed => value,
                    DiscountType::Percentage => value * p.price / 100.0,
                    DiscountType::Price => p.price - value,
                    DiscountType::FreeItem => 0.0,
                };
            }
            Some(MappedItem {
                product_id: p.id,
                price: p.price,
                quantity: original.quantity,
                discount_id: d.map(|d| d.id),
                discount_amount,
                final_price: p.price - discount_amount,
            })
        })
        .collect()
}

fn totals(items: &[MappedItem]) -> (f64, f64, f64) {
    let full = items
        .iter()
        .map(|i| i.price * f64::from(i.quantity))
        .sum::<f64>();
    let discounted = items
        .iter()
        .map(|i| i.final_price * f64::from(i.quantity))
        .sum::<f64>();
    (full, discounted, full - discounted)
}

fn apply_voucher(vouchers: &[voucher::Model], group: VoucherGroup, price: f64) -> f64 {
    for v in vouchers {
        if v.group != group {
            continue;
        }
        let Some(value) = v.value.filter(|v| *v != 0.0) else {
            continue;
        };
        return match v.r#type {
            VoucherType::Fixed => price - value,
            VoucherType::Percentage => price - price * value / 100.0,
            VoucherType::FreeItem => continue,
        };
    }
    price
}

/// Execute in a PostgreSQL transaction. Dropping an uncommitted SeaORM
/// transaction rolls back every stock/cart/discount/voucher write.
pub async fn execute(state: &AppState, input: CheckoutInput) -> AppResult<Value> {
    input.validate()?;
    let tx = state.db.begin().await.map_err(AppError::db)?;
    let now = Utc::now().naive_utc();
    // Order-specific locking must serialize concurrent checkouts for the user.
    user::Entity::find_by_id(input.order.user_id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("User"))?;
    address::Entity::find_by_id(input.order.address_id)
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Address"))?;

    let product_ids = input
        .order_items
        .iter()
        .map(|i| i.product_id)
        .collect::<Vec<_>>();
    let products = product::Entity::find()
        .filter(product::Column::Id.is_in(product_ids.clone()))
        .all(&tx)
        .await
        .map_err(AppError::db)?;
    for id in &product_ids {
        if !products.iter().any(|p| p.id == *id) {
            return Err(not_found(&id.to_string()));
        }
    }
    let cart = cart::Entity::find()
        .filter(cart::Column::UserId.eq(input.order.user_id))
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Cart"))?;

    let voucher_ids = input.order.voucher_ids.clone().unwrap_or_default();
    let vouchers = if voucher_ids.is_empty() {
        Vec::new()
    } else {
        voucher::Entity::find()
            .filter(voucher::Column::Id.is_in(voucher_ids.clone()))
            .all(&tx)
            .await
            .map_err(AppError::db)?
    };
    for id in &voucher_ids {
        let v = vouchers
            .iter()
            .find(|v| v.id == *id)
            .ok_or_else(|| not_found("Voucher not found or already gone 👻"))?;
        if v.end_date < now {
            return Err(AppError::Conflict(format!(
                "\"{}\" is already expired 😬",
                v.name
            )));
        }
    }

    let user_vouchers = if voucher_ids.is_empty() {
        Vec::new()
    } else {
        user_voucher::Entity::find()
            .filter(user_voucher::Column::UserId.eq(input.order.user_id))
            .filter(user_voucher::Column::VoucherId.is_in(voucher_ids.clone()))
            .all(&tx)
            .await
            .map_err(AppError::db)?
    };
    for id in &voucher_ids {
        let uv = user_vouchers
            .iter()
            .find(|v| v.voucher_id == *id)
            .ok_or_else(|| not_found("Voucher isn't linked to your account 😶‍🌫️"))?;
        if uv.status != UserVoucherStatus::Claimed {
            return Err(AppError::Conflict(format!(
                "Voucher {id} can't be used anymore 😵"
            )));
        }
        if uv.expired_at < now {
            return Err(AppError::Conflict(format!(
                "\"{id}\" is already expired 😬"
            )));
        }
    }

    let discount_ids = input
        .order_items
        .iter()
        .filter_map(|i| i.discount_id)
        .collect::<Vec<_>>();
    let mut product_quantities: BTreeMap<Uuid, i64> = BTreeMap::new();
    let mut discount_quantities: BTreeMap<Uuid, i64> = BTreeMap::new();
    for item in &input.order_items {
        *product_quantities.entry(item.product_id).or_default() += i64::from(item.quantity);
        if let Some(id) = item.discount_id {
            *discount_quantities.entry(id).or_default() += i64::from(item.quantity);
        }
    }
    let discounts = if discount_ids.is_empty() {
        Vec::new()
    } else {
        // The legacy raw SQL used "discount" (singular), while Prisma maps
        // the actual table to "discounts". Use the real table so checkout works.
        discount::Entity::find()
            .filter(discount::Column::Id.is_in(discount_ids.clone()))
            .order_by_asc(discount::Column::Id)
            .lock_exclusive()
            .all(&tx)
            .await
            .map_err(AppError::db)?
    };
    for item in &input.order_items {
        let Some(id) = item.discount_id else {
            continue;
        };
        let d = discounts
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| not_found(&format!("Discount \"{id}\" not found 😶‍🌫️")))?;
        if d.redeemed_count >= d.max_redeemed
            || item.quantity > d.max_redeemed
            || i64::from(d.redeemed_count) + discount_quantities[&id] > i64::from(d.max_redeemed)
        {
            return Err(AppError::Conflict(format!(
                "Discount \"{}\" is maxed out 🚷",
                d.name
            )));
        }
        if d.start_date > now {
            return Err(AppError::Conflict(format!(
                "Discount \"{}\" hasn't started yet",
                d.name
            )));
        }
        if d.end_date < now {
            return Err(AppError::Conflict(format!(
                "Discount \"{}\" is expired 💀",
                d.name
            )));
        }
    }

    // Lock inventory rows before cart items, matching checkout steps order.
    let inventories = inventory::Entity::find()
        .filter(inventory::Column::ProductId.is_in(product_ids.clone()))
        .order_by_asc(inventory::Column::ProductId)
        .lock_exclusive()
        .all(&tx)
        .await
        .map_err(AppError::db)?;
    for item in &input.order_items {
        let inv = inventories
            .iter()
            .find(|i| i.product_id == item.product_id)
            .ok_or_else(|| {
                not_found(&format!(
                    "Inventory for product \"{}\" not found 🫠",
                    item.product_id
                ))
            })?;
        if i64::from(inv.quantity) - i64::from(inv.reserved_quantity)
            < product_quantities[&item.product_id]
        {
            return Err(AppError::Conflict(format!(
                "\"{}\" is running out of stock 😩",
                item.product_id
            )));
        }
    }
    let cart_items = cart_item::Entity::find()
        .filter(cart_item::Column::CartId.eq(cart.id))
        .filter(cart_item::Column::ProductId.is_in(product_ids.clone()))
        .order_by_asc(cart_item::Column::ProductId)
        .lock_exclusive()
        .all(&tx)
        .await
        .map_err(AppError::db)?;
    let missing = product_ids
        .iter()
        .filter(|id| !cart_items.iter().any(|i| i.product_id == **id))
        .map(Uuid::to_string)
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(not_found(&format!(
            "Some cart items not found: {} 🫥",
            missing.join(", ")
        )));
    }

    if !discount_ids.is_empty() {
        let links = product_discount::Entity::find()
            .filter(product_discount::Column::ProductId.is_in(product_ids.clone()))
            .filter(product_discount::Column::DiscountId.is_in(discount_ids))
            .all(&tx)
            .await
            .map_err(AppError::db)?;
        // This conditional is unusual but is exactly how Nest's
        // HandleProductsDiscountOrder validates the relation.
        if links.len() == input.order_items.len() {
            for item in &input.order_items {
                if !links.iter().any(|r| r.product_id == item.product_id) {
                    return Err(not_found(&format!(
                        "No product-discount relation found for product \"{}\" and discount \"{}\"",
                        item.product_id,
                        item.discount_id
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "undefined".into())
                    )));
                }
            }
        }
    }

    // Nest's update steps deliberately reserve inventory, increment redeemed
    // counters, and decrement (rather than delete) cart item quantities.
    for (product_id, quantity) in &product_quantities {
        let inv = inventories
            .iter()
            .find(|i| i.product_id == *product_id)
            .expect("validated inventory");
        let mut row: inventory::ActiveModel = inv.clone().into();
        row.reserved_quantity = Set(i32::try_from(i64::from(inv.reserved_quantity) + *quantity)
            .map_err(|_| AppError::Conflict("Inventory reservation overflow".into()))?);
        row.updated_at = Set(now);
        row.update(&tx).await.map_err(AppError::db)?;
    }
    for (id, quantity) in &discount_quantities {
        let d = discounts
            .iter()
            .find(|d| d.id == *id)
            .expect("validated discount");
        // Aggregate duplicate discount lines before updating a locked row.
        let mut row: discount::ActiveModel = d.clone().into();
        row.redeemed_count = Set(i32::try_from(i64::from(d.redeemed_count) + *quantity)
            .map_err(|_| AppError::Conflict("Discount usage overflow".into()))?);
        row.updated_at = Set(now);
        row.update(&tx).await.map_err(AppError::db)?;
    }
    for (product_id, quantity) in &product_quantities {
        let ci = cart_items
            .iter()
            .find(|i| i.product_id == *product_id)
            .expect("validated cart item");
        let mut row: cart_item::ActiveModel = ci.clone().into();
        row.quantity = Set(i32::try_from(i64::from(ci.quantity) - *quantity)
            .map_err(|_| AppError::Conflict("Cart quantity underflow".into()))?);
        row.updated_at = Set(now);
        row.update(&tx).await.map_err(AppError::db)?;
    }
    for uv in &user_vouchers {
        let mut row: user_voucher::ActiveModel = uv.clone().into();
        row.status = Set(UserVoucherStatus::Used);
        row.updated_at = Set(now);
        row.update(&tx).await.map_err(AppError::db)?;
    }

    let mapped = map_items(&products, &discounts, &input.order_items);
    let (total_price, total_final_price, total_discount) = totals(&mapped);
    let shipping_cost = apply_voucher(&vouchers, VoucherGroup::Shipping, input.shipment.cost);
    let discounted_total = apply_voucher(&vouchers, VoucherGroup::Checkout, total_final_price);
    let order_price = discounted_total + shipping_cost;

    let order_id = Uuid::new_v4();
    order::ActiveModel {
        id: Set(order_id),
        user_id: Set(input.order.user_id),
        address_id: Set(input.order.address_id),
        voucher_amount: Set(0.0),
        discount_amount: Set(total_discount),
        total_price: Set(total_price),
        final_price: Set(order_price),
        status: Set(order::OrderStatus::Pending),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&tx)
    .await
    .map_err(AppError::db)?;

    for item in &mapped {
        order_item::ActiveModel {
            id: Set(Uuid::new_v4()),
            order_id: Set(order_id),
            product_id: Set(item.product_id),
            discount_id: Set(item.discount_id),
            discount_amount: Set(item.discount_amount),
            price: Set(item.price),
            final_price: Set(item.final_price),
            quantity: Set(item.quantity),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&tx)
        .await
        .map_err(AppError::db)?;
    }
    // Nest creates FREE_ITEM voucher rows before FREE_ITEM discount rows.
    for v in &vouchers {
        if v.r#type != VoucherType::FreeItem {
            continue;
        }
        let product_id = v
            .product_id
            .ok_or_else(|| AppError::BadRequest("Free item voucher requires productId".into()))?;
        let quantity = v
            .quantity
            .ok_or_else(|| AppError::BadRequest("Free item voucher requires quantity".into()))?;
        insert_free_item(&tx, order_id, product_id, quantity, now).await?;
    }
    for d in &discounts {
        if d.discount_type != DiscountType::FreeItem {
            continue;
        }
        let product_id = d
            .product_id
            .ok_or_else(|| AppError::BadRequest("Free item discount requires productId".into()))?;
        let quantity = d
            .quantity
            .ok_or_else(|| AppError::BadRequest("Free item discount requires quantity".into()))?;
        insert_free_item(&tx, order_id, product_id, quantity, now).await?;
    }
    for id in voucher_ids {
        order_voucher::ActiveModel {
            order_id: Set(order_id),
            voucher_id: Set(id),
        }
        .insert(&tx)
        .await
        .map_err(AppError::db)?;
    }
    payment::ActiveModel {
        id: Set(Uuid::new_v4()),
        order_id: Set(order_id),
        method: Set(input.payment.method.clone()),
        method_type: Set(input.payment.method),
        amount: Set(order_price),
        status: Set(payment::PaymentStatus::Pending),
        transaction_code: Set(None),
        transaction_id: Set(None),
        expiry_date: Set(None),
        paid_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&tx)
    .await
    .map_err(AppError::db)?;
    shipment::ActiveModel {
        id: Set(Uuid::new_v4()),
        order_id: Set(order_id),
        courier: Set(input.shipment.courier),
        service: Set(input.shipment.service),
        cost: Set(shipping_cost),
        estimated: Set(super::super::catalog_common::parse_datetime(
            &input.shipment.estimated,
        )?),
        delivered_at: Set(None),
        description: Set(input.shipment.description),
        tracking_number: Set(None),
        status: Set(shipment::ShipmentStatus::Pending),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&tx)
    .await
    .map_err(AppError::db)?;

    tx.commit().await.map_err(AppError::db)?;
    Ok(success("created", "Order"))
}

async fn insert_free_item(
    tx: &sea_orm::DatabaseTransaction,
    order_id: Uuid,
    product_id: Uuid,
    quantity: i32,
    now: chrono::NaiveDateTime,
) -> AppResult<()> {
    order_item::ActiveModel {
        id: Set(Uuid::new_v4()),
        order_id: Set(order_id),
        product_id: Set(product_id),
        discount_id: Set(None),
        discount_amount: Set(0.0),
        price: Set(0.0),
        final_price: Set(0.0),
        quantity: Set(quantity),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(tx)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::voucher::VoucherType;
    #[test]
    fn totals_are_per_unit_then_multiplied_by_quantity() {
        let product_id = Uuid::new_v4();
        let item = MappedItem {
            product_id,
            price: 100.0,
            quantity: 3,
            discount_id: None,
            discount_amount: 20.0,
            final_price: 80.0,
        };
        assert_eq!(totals(&[item]), (300.0, 240.0, 60.0));
    }
    #[test]
    fn free_item_voucher_does_not_change_checkout_price() {
        let now = Utc::now().naive_utc();
        let voucher = voucher::Model {
            id: Uuid::new_v4(),
            code: "FREE".into(),
            name: "Free".into(),
            description: None,
            r#type: VoucherType::FreeItem,
            group: VoucherGroup::Checkout,
            value: Some(50.0),
            product_id: None,
            quantity: None,
            minimum_purchase: 0.0,
            is_active: true,
            start_date: now,
            end_date: now,
            max_redeemed: 100,
            redeemed_count: 0,
            created_at: now,
            updated_at: now,
        };
        assert_eq!(
            apply_voucher(&[voucher], VoucherGroup::Checkout, 250.0),
            250.0
        );
    }
}
