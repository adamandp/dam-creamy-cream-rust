use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
    routing::get,
};
use chrono::Utc;
use sea_orm::sea_query::extension::postgres::PgExpr;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    authorization,
    catalog_common::{collection, reject_no_change, require_positive, require_text, timestamps},
};
use crate::{
    common::{Pagination, not_found, parse_uuid, success},
    entity::{
        category,
        discount::{self, DiscountType},
        product, product_discount, review,
    },
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Create {
    category_id: Uuid,
    discount_id: Option<Uuid>,
    name: String,
    description: Option<String>,
    price: f64,
    is_active: bool,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    category_id: Option<Uuid>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    discount_id: Option<Option<Uuid>>,
    name: Option<String>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    description: Option<Option<String>>,
    price: Option<f64>,
    is_active: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogQuery {
    q: Option<String>,
    sort: Option<String>,
    cat: Option<String>,
    min: Option<f64>,
    max: Option<f64>,
    #[serde(flatten)]
    pagination: Pagination,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Presign {
    file_name: String,
    file_size: i64,
    mime_type: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Confirm {
    key: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/catalog", get(catalog))
        .route("/{id}", axum::routing::patch(update).delete(remove))
        .route("/{id}/image/presign", axum::routing::post(presign))
        .route("/{id}/image/confirm", axum::routing::post(confirm))
}

pub fn current_image_url(stored: Option<String>, id: Uuid, public_base: &str) -> Option<String> {
    stored.map(|url| {
        if let Some(asset) = url
            .rsplit_once(&format!("/media/products/{id}/"))
            .map(|(_, s)| s)
        {
            if Uuid::parse_str(asset).is_ok() {
                return format!("{public_base}/media/products/{id}/{asset}");
            }
        }
        url
    })
}

pub fn image_key(url: &str, id: Uuid) -> Option<String> {
    let asset = url.rsplit_once(&format!("/media/products/{id}/"))?.1;
    Uuid::parse_str(asset).ok()?;
    Some(format!("products/{id}/{asset}"))
}

pub fn body(m: product::Model, public_base: &str) -> Value {
    let (created_at, updated_at) = timestamps(m.created_at, m.updated_at);
    let image_url = current_image_url(m.image_url, m.id, public_base);
    json!({"id":m.id,"categoryId":m.category_id,"discountId":m.discount_id,"name":m.name,"imageUrl":image_url,"description":m.description,"price":m.price,"isActive":m.is_active,"createdAt":created_at,"updatedAt":updated_at})
}

pub fn price_with_discount(
    price: f64,
    discount: Option<&discount::Model>,
) -> (Option<f64>, Option<DiscountType>, Option<f64>) {
    let Some(d) = discount.filter(|d| d.value.unwrap_or(0.0) != 0.0) else {
        return (None, None, None);
    };
    let value = d.value.unwrap_or_default();
    let amount = match &d.discount_type {
        DiscountType::Fixed => value,
        DiscountType::Percentage => value * price / 100.0,
        _ => 0.0,
    };
    (
        Some(price - amount),
        Some(d.discount_type.clone()),
        Some(value),
    )
}

pub async fn active_discount(
    state: &AppState,
    product_id: Uuid,
) -> AppResult<Option<discount::Model>> {
    let link = product_discount::Entity::find()
        .filter(product_discount::Column::ProductId.eq(product_id))
        .filter(product_discount::Column::IsActive.eq(true))
        .one(&state.db)
        .await
        .map_err(AppError::db)?;
    match link {
        Some(link) => discount::Entity::find_by_id(link.discount_id)
            .one(&state.db)
            .await
            .map_err(AppError::db),
        None => Ok(None),
    }
}

fn validate(name: &str, description: Option<&str>, price: f64) -> AppResult<()> {
    require_text(name, 1, 100, "name")?;
    if let Some(d) = description {
        require_text(d, 1, 255, "description")?;
    }
    require_positive(price, 1000.0, 1_000_000.0, "price")
}

async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Create>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    validate(&b.name, b.description.as_deref(), b.price)?;
    if category::Entity::find_by_id(b.category_id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("category"));
    }
    let now = Utc::now().naive_utc();
    product::ActiveModel {
        id: Set(Uuid::new_v4()),
        category_id: Set(b.category_id),
        discount_id: Set(b.discount_id),
        name: Set(b.name),
        image_url: Set(None),
        description: Set(b.description),
        price: Set(b.price),
        is_active: Set(b.is_active),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&s.db)
    .await
    .map_err(AppError::db)?;
    Ok((StatusCode::CREATED, Json(success("created", "Product"))))
}

async fn list(
    State(s): State<AppState>,
    ApiQuery(q): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = q.checked()?;
    let total = product::Entity::find()
        .count(&s.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("Product"));
    }
    let data = product::Entity::find()
        .order_by_asc(product::Column::CreatedAt)
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(collection(
        "Product",
        data.into_iter()
            .map(|m| body(m, &s.config.public_base_url))
            .collect(),
        page,
        limit,
        total,
    )))
}

async fn catalog(
    State(s): State<AppState>,
    ApiQuery(q): ApiQuery<CatalogQuery>,
) -> AppResult<Json<Value>> {
    let (page, limit) = q.pagination.checked()?;
    if let Some(v) = &q.q {
        require_text(v, 1, 100, "q")?;
    }
    if let Some(v) = &q.sort {
        if !matches!(v.as_str(), "asc" | "desc" | "default") {
            return Err(AppError::BadRequest("Invalid sort".into()));
        }
    }
    if let Some(v) = &q.cat {
        require_text(v, 1, 100, "cat")?;
    }
    if let Some(v) = q.min {
        require_positive(v, 0.0, 1_000_000.0, "min")?;
    }
    if let Some(v) = q.max {
        require_positive(v, 0.0, 1_000_000.0, "max")?;
    }
    let mut query = product::Entity::find().filter(product::Column::IsActive.eq(true));
    if let Some(q) = &q.q {
        let pattern = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
        query = query.filter(
            Condition::any()
                .add(sea_orm::sea_query::Expr::col(product::Column::Name).ilike(pattern.clone()))
                .add(sea_orm::sea_query::Expr::col(product::Column::Description).ilike(pattern)),
        );
    }
    if let Some(cat) = &q.cat {
        let ids = cat
            .split(',')
            .map(parse_uuid)
            .collect::<AppResult<Vec<_>>>()?;
        query = query.filter(product::Column::CategoryId.is_in(ids));
    }
    if let Some(min) = q.min {
        query = query.filter(product::Column::Price.gte(min));
    }
    if let Some(max) = q.max {
        query = query.filter(product::Column::Price.lte(max));
    }
    let total = query.clone().count(&s.db).await.map_err(AppError::db)?;
    if total == 0 && page == 1 {
        return Err(not_found("Product"));
    }
    query = match q.sort.as_deref() {
        Some("asc") => query.order_by_asc(product::Column::Price),
        Some("desc") => query.order_by_desc(product::Column::Price),
        _ => query.order_by_desc(product::Column::CreatedAt),
    };
    let models = query
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&s.db)
        .await
        .map_err(AppError::db)?;
    let mut data = Vec::with_capacity(models.len());
    for model in models {
        let reviews = review::Entity::find()
            .filter(review::Column::ProductId.eq(model.id))
            .all(&s.db)
            .await
            .map_err(AppError::db)?;
        let rate = if reviews.is_empty() {
            0.0
        } else {
            reviews.iter().map(|r| f64::from(r.rating)).sum::<f64>() / reviews.len() as f64
        };
        let discount = active_discount(&s, model.id).await?;
        let (discount_price, discount_type, discount_value) =
            price_with_discount(model.price, discount.as_ref());
        data.push(json!({"id":model.id,"name":model.name,"imageUrl":current_image_url(model.image_url,model.id,&s.config.public_base_url).unwrap_or_default(),"description":model.description.unwrap_or_default(),"rate":rate,"price":model.price,"discountPrice":discount_price,"discountType":discount_type,"discountValue":discount_value}));
    }
    Ok(Json(collection("Product", data, page, limit, total)))
}

async fn update(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Update>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let m = product::Entity::find_by_id(parse_uuid(&id)?)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Product"))?;
    validate(
        b.name.as_deref().unwrap_or(&m.name),
        b.description
            .as_ref()
            .map(|v| v.as_deref())
            .unwrap_or(m.description.as_deref()),
        b.price.unwrap_or(m.price),
    )?;
    if let Some(cat) = b.category_id {
        if category::Entity::find_by_id(cat)
            .one(&s.db)
            .await
            .map_err(AppError::db)?
            .is_none()
        {
            return Err(not_found("category"));
        }
    }
    reject_no_change(&[
        b.category_id.map(|v| v == m.category_id),
        b.discount_id.as_ref().map(|v| v == &m.discount_id),
        b.name.as_ref().map(|v| v == &m.name),
        b.description.as_ref().map(|v| v == &m.description),
        b.price.map(|v| v == m.price),
        b.is_active.map(|v| v == m.is_active),
    ])?;
    let mut a: product::ActiveModel = m.into();
    if let Some(v) = b.category_id {
        a.category_id = Set(v);
    }
    if let Some(v) = b.discount_id {
        a.discount_id = Set(v);
    }
    if let Some(v) = b.name {
        a.name = Set(v);
    }
    if let Some(v) = b.description {
        a.description = Set(v);
    }
    if let Some(v) = b.price {
        a.price = Set(v);
    }
    if let Some(v) = b.is_active {
        a.is_active = Set(v);
    }
    a.updated_at = Set(Utc::now().naive_utc());
    a.update(&s.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Product")))
}

async fn remove(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let id = parse_uuid(&id)?;
    let previous = product::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Product"))?;
    product::Entity::delete_by_id(id)
        .exec(&s.db)
        .await
        .map_err(AppError::db)?;
    if let Some(key) = previous.image_url.as_deref().and_then(|v| image_key(v, id)) {
        if let Err(err) = s.storage.delete_image(&key).await {
            tracing::warn!(error=%err,"product image cleanup failed");
        }
    }
    Ok(Json(success("deleted", "Product")))
}

async fn presign(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Presign>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let id = parse_uuid(&id)?;
    if product::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .is_none()
    {
        return Err(not_found("Product"));
    }
    require_text(&b.file_name, 1, 255, "fileName")?;
    if !(1..=2_097_152).contains(&b.file_size) {
        return Err(AppError::BadRequest(
            "Image size must be at most 2 MiB".into(),
        ));
    }
    if !matches!(
        b.mime_type.as_str(),
        "image/jpeg" | "image/png" | "image/webp"
    ) {
        return Err(AppError::BadRequest(
            "Only JPEG, PNG or WebP images are supported".into(),
        ));
    }
    let key = format!("pending/products/{id}/{}", Uuid::new_v4());
    let url = s
        .storage
        .presign_put(&key, &b.mime_type, b.file_size)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"data":{"uploadUrl":url,"key":key,"expiresIn":600,"headers":{"Content-Type":b.mime_type}}}),
        ),
    ))
}

async fn confirm(
    State(s): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<Confirm>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&s, &headers)
        .await?
        .require_admin()?;
    let id = parse_uuid(&id)?;
    let asset = b
        .key
        .strip_prefix(&format!("pending/products/{id}/"))
        .ok_or_else(|| AppError::BadRequest("Invalid image object key".into()))?;
    parse_uuid(asset)?;
    let image_url = format!("{}/media/products/{id}/{asset}", s.config.public_base_url);
    if image_url.len() > 255 {
        return Err(AppError::BadRequest(
            "PUBLIC_BASE_URL is too long for image_url column".into(),
        ));
    }
    let final_key = format!("products/{id}/{asset}");
    let outcome: AppResult<Option<String>> = async {
        let tx = s.db.begin().await.map_err(AppError::db)?;
        let row = product::Entity::find_by_id(id)
            .lock_exclusive()
            .one(&tx)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| not_found("Product"))?;
        if row
            .image_url
            .as_deref()
            .and_then(|url| image_key(url, id))
            .as_deref()
            == Some(final_key.as_str())
        {
            tx.commit().await.map_err(AppError::db)?;
            return Ok(None);
        }
        let etag = s.storage.verify_image(&b.key).await?;
        let previous = row.image_url.as_deref().and_then(|url| image_key(url, id));
        s.storage
            .copy_verified_image(&b.key, &final_key, &etag)
            .await?;
        let mut active: product::ActiveModel = row.into();
        active.image_url = Set(Some(image_url));
        active.updated_at = Set(Utc::now().naive_utc());
        active.update(&tx).await.map_err(AppError::db)?;
        tx.commit().await.map_err(AppError::db)?;
        Ok(previous)
    }
    .await;
    let previous = match outcome {
        Ok(v) => v,
        Err(e) => {
            if let Err(cleanup) = delete_if_unreferenced(&s, id, &final_key).await {
                tracing::warn!(error=%cleanup,"could not clean failed product upload");
            }
            return Err(e);
        }
    };
    for key in [Some(b.key), previous]
        .into_iter()
        .flatten()
        .filter(|key| key != &final_key)
    {
        if let Err(e) = delete_if_unreferenced(&s, id, &key).await {
            tracing::warn!(error=%e,"could not clean obsolete product upload");
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(success("updated", "Product Image")),
    ))
}

async fn delete_if_unreferenced(s: &AppState, id: Uuid, key: &str) -> AppResult<()> {
    let tx = s.db.begin().await.map_err(AppError::db)?;
    let product = product::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&tx)
        .await
        .map_err(AppError::db)?;
    let is_active = product
        .as_ref()
        .and_then(|m| m.image_url.as_deref())
        .and_then(|v| image_key(v, id))
        .as_deref()
        == Some(key);
    if !is_active {
        s.storage.delete_image(key).await?;
    }
    tx.commit().await.map_err(AppError::db)
}

pub async fn get_image(
    State(s): State<AppState>,
    Path((id, asset)): Path<(String, String)>,
) -> AppResult<Redirect> {
    let id = parse_uuid(&id)?;
    parse_uuid(&asset)?;
    let m = product::Entity::find_by_id(id)
        .one(&s.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Product"))?;
    let key = format!("products/{id}/{asset}");
    if m.image_url
        .as_deref()
        .and_then(|v| image_key(v, id))
        .as_deref()
        != Some(key.as_str())
    {
        return Err(not_found("Product Image"));
    }
    let url = s.storage.presign_get(&key).await?;
    Ok(Redirect::temporary(&url))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discount_price_matches_legacy_calculation() {
        assert_eq!(price_with_discount(50_000.0, None), (None, None, None));
        let now = Utc::now().naive_utc();
        let d = discount::Model {
            id: Uuid::new_v4(),
            name: "test".into(),
            discount_type: DiscountType::Percentage,
            value: Some(10.0),
            max_applied: 1,
            max_redeemed: 1,
            redeemed_count: 0,
            description: None,
            product_id: None,
            quantity: None,
            start_date: now,
            end_date: now,
            created_at: now,
            updated_at: now,
        };
        assert_eq!(price_with_discount(50_000.0, Some(&d)).0, Some(45_000.0));
    }
    #[test]
    fn image_url_mapping_only_rebases_owned_keys() {
        let id = Uuid::new_v4();
        let a = Uuid::new_v4();
        let url = format!("https://old.com/media/products/{id}/{a}");
        assert_eq!(
            current_image_url(Some(url), id, "https://api.test"),
            Some(format!("https://api.test/media/products/{id}/{a}"))
        );
        assert!(image_key("https://cdn.example/test.jpg", id).is_none());
    }
}
