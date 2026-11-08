use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde_json::{Value, json};
use tokio::{
    net::TcpStream,
    time::{Duration, timeout},
};

use crate::{
    error::{AppError, AppResult},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(external)).route("/db", get(database))
}

// The legacy health endpoint checks the public google.com host. Keep this
// probe out of the readiness path for dependencies running in private networks.
async fn external() -> (StatusCode, Json<Value>) {
    let available = timeout(Duration::from_secs(5), TcpStream::connect(("google.com", 443)))
        .await
        .is_ok_and(|attempt| attempt.is_ok());
    if available {
        let item = json!({"status":"up"});
        (
            StatusCode::OK,
            Json(json!({"status":"ok","info":{"google":item},"error":{},"details":{"google":item}})),
        )
    } else {
        let item = json!({"status":"down","message":"External health check failed"});
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status":"error","info":{},"error":{"google":item},"details":{"google":item}})),
        )
    }
}

async fn database(State(state): State<AppState>) -> AppResult<Json<Value>> {
    state.db
        .execute(Statement::from_string(DbBackend::Postgres, "SELECT 1"))
        .await
        .map_err(AppError::db)?;
    Ok(Json(json!({"db":{"status":"up"}})))
}
