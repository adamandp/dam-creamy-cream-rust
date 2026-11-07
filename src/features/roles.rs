use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::Deserialize;
use serde_json::{Value, json};
use validator::Validate;

use super::authorization;
use crate::{
    common::{Pagination, not_found, paging, parse_uuid, success, success_verb},
    entity::role,
    error::{AppError, AppResult},
    extract::{ApiJson, ApiQuery},
    state::AppState,
};

#[derive(Deserialize, Validate)]
pub struct RoleBody {
    #[validate(length(min = 1, max = 50))]
    name: String,
}

fn validate(body: &RoleBody) -> AppResult<()> {
    body.validate()
        .map_err(|err| AppError::BadRequest(err.to_string()))
}

fn response(model: role::Model) -> Value {
    json!({"id": model.id, "name": model.name})
}

fn is_builtin_role(name: &str) -> bool {
    matches!(name, "customer" | "admin" | "super-admin")
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(find_all).post(create))
        .route("/name/{name}", get(find_by_name))
        .route("/{id}", get(find_one).patch(update).delete(remove))
}

async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<RoleBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_super_admin()?;
    validate(&body)?;
    let now = Utc::now().naive_utc();
    role::ActiveModel {
        id: Set(uuid::Uuid::new_v4()),
        name: Set(body.name),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .map_err(AppError::db)?;
    Ok((StatusCode::CREATED, Json(success("created", "Role"))))
}

async fn find_all(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<Pagination>,
) -> AppResult<Json<Value>> {
    let (page, limit) = query.checked()?;
    let total = role::Entity::find()
        .count(&state.db)
        .await
        .map_err(AppError::db)?;
    if total == 0 {
        return Err(not_found("Role"));
    }
    let items = role::Entity::find()
        .order_by_asc(role::Column::CreatedAt)
        .order_by_asc(role::Column::Id)
        .offset((page - 1) * limit)
        .limit(limit)
        .all(&state.db)
        .await
        .map_err(AppError::db)?;
    Ok(Json(json!({
        // Intentionally matches the legacy RolesService response string.
        "message": success_verb("retrieved", "Category"),
        "data": items.into_iter().map(response).collect::<Vec<_>>(),
        "paging": paging(page, limit, total),
    })))
}

async fn find_one(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let model = role::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Role"))?;
    Ok(Json(
        json!({"message": success_verb("retrieved", "Role"), "data": response(model)}),
    ))
}

async fn find_by_name(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> AppResult<Json<Value>> {
    let model = role::Entity::find()
        .filter(role::Column::Name.eq(name))
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Role"))?;
    Ok(Json(
        json!({"message": success_verb("retrieved", "Role"), "data": response(model)}),
    ))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<RoleBody>,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_super_admin()?;
    validate(&body)?;
    let model = role::Entity::find_by_id(parse_uuid(&id)?)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Role"))?;
    if body.name != model.name && (is_builtin_role(&body.name) || is_builtin_role(&model.name)) {
        return Err(AppError::Forbidden(
            "Built-in role names cannot be changed".into(),
        ));
    }
    let mut active: role::ActiveModel = model.into();
    active.name = Set(body.name);
    active.updated_at = Set(Utc::now().naive_utc());
    active.update(&state.db).await.map_err(AppError::db)?;
    Ok(Json(success("updated", "Role")))
}

async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    authorization::authenticate(&state, &headers)
        .await?
        .require_super_admin()?;
    let id = parse_uuid(&id)?;
    let role = role::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| not_found("Role"))?;
    if is_builtin_role(&role.name) {
        return Err(AppError::Forbidden(
            "Built-in roles cannot be deleted".into(),
        ));
    }
    let result = role::Entity::delete_by_id(id)
        .exec(&state.db)
        .await
        .map_err(AppError::db)?;
    if result.rows_affected == 0 {
        return Err(not_found("Role"));
    }
    Ok(Json(success("deleted", "Role")))
}
