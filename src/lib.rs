pub mod common;
pub mod config;
pub mod entity;
pub mod error;
pub mod features;
pub mod infrastructure;
pub mod state;

use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::{get, post},
};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

use state::AppState;

pub fn router(state: AppState) -> Router {
    let origins = if state.config.frontend_origins.is_empty() {
        AllowOrigin::mirror_request()
    } else {
        AllowOrigin::list(
            state
                .config
                .frontend_origins
                .iter()
                .filter_map(|origin| origin.parse::<HeaderValue>().ok())
                .collect::<Vec<_>>(),
        )
    };
    Router::new()
        .nest("/auth", features::auth::routes())
        .route("/sessions/refresh", post(features::session::refresh))
        .nest("/roles", features::roles::routes())
        .nest("/users", features::users::routes())
        .nest("/addresses", features::addresses::routes())
        .route("/media/users/{id}/{asset}", get(features::users::get_image))
        .layer(TraceLayer::new_for_http())
        .layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_credentials(true)
                .allow_headers([header::AUTHORIZATION, header::ACCEPT, header::CONTENT_TYPE])
                .allow_methods([
                    Method::GET,
                    Method::HEAD,
                    Method::POST,
                    Method::PUT,
                    Method::PATCH,
                    Method::DELETE,
                    Method::OPTIONS,
                ]),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use chrono::Utc;
    use sea_orm::{DbBackend, MockDatabase};
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::{
        config::Config,
        entity::{address, role, user},
        infrastructure::storage::Storage,
    };

    fn config() -> Config {
        Config {
            port: 3000,
            database_url: String::new(),
            redis_url: "redis://localhost/".into(),
            access_jwt_secret: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            refresh_jwt_secret: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            public_base_url: "http://localhost:3000".into(),
            frontend_origins: vec!["http://localhost:5173".into()],
            minio_endpoint: "http://localhost:9000".into(),
            minio_access_key: "test".into(),
            minio_secret_key: "test".into(),
            minio_bucket: "test".into(),
            minio_region: "us-east-1".into(),
            production: false,
        }
    }

    fn app_with_queries<T, I>(results: I) -> Router
    where
        T: sea_orm::IntoMockRow,
        I: IntoIterator<Item = Vec<T>>,
    {
        let config = config();
        let database = MockDatabase::new(DbBackend::Postgres)
            .append_query_results(results)
            .into_connection();
        router(AppState {
            db: database,
            redis: redis::Client::open(config.redis_url.clone()).unwrap(),
            storage: Storage::offline(&config),
            config,
        })
    }

    async fn request_method(
        app: Router,
        path: &str,
        method: axum::http::Method,
    ) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    async fn request(app: Router, path: &str) -> (StatusCode, Value) {
        request_method(app, path, axum::http::Method::GET).await
    }

    #[tokio::test]
    async fn roles_retain_legacy_response_shape() {
        let now = Utc::now().naive_utc();
        let id = uuid::Uuid::new_v4();
        let model = role::Model {
            id,
            name: "customer".into(),
            created_at: now,
            updated_at: now,
        };
        let (status, body) =
            request(app_with_queries(vec![vec![model]]), &format!("/roles/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["name"], "customer");
        assert_eq!(body["data"]["id"], id.to_string());
        assert!(body["data"].get("createdAt").is_none());
    }

    #[tokio::test]
    async fn user_detail_omits_password() {
        let now = Utc::now().naive_utc();
        let id = uuid::Uuid::new_v4();
        let model = user::Model {
            id,
            role_id: uuid::Uuid::new_v4(),
            username: "DummyUser".into(),
            email: None,
            phone_number: None,
            password: "secret-hash".into(),
            full_name: "Dummy User".into(),
            birth_date: None,
            image_url: None,
            gender: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        let (status, body) =
            request(app_with_queries(vec![vec![model]]), &format!("/users/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].get("password").is_none());
        assert_eq!(body["data"]["fullName"], "Dummy User");
    }

    #[tokio::test]
    async fn address_detail_maps_snake_case_columns_to_json_contract() {
        let now = Utc::now().naive_utc();
        let id = uuid::Uuid::new_v4();
        let model = address::Model {
            id,
            user_id: uuid::Uuid::new_v4(),
            recipient_name: "Adam".into(),
            recipient_phone_number: "081234567890".into(),
            province: "Banten".into(),
            city: "Tangerang".into(),
            subdistrict: "BSD".into(),
            village: "BSD".into(),
            postal_code: "15310".into(),
            address: "Example".into(),
            is_primary: true,
            created_at: now,
            updated_at: now,
        };
        let (status, body) = request(
            app_with_queries(vec![vec![model]]),
            &format!("/addresses/{id}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["recipientPhoneNumber"], "081234567890");
        assert_eq!(body["data"]["isPrimary"], true);
    }

    #[tokio::test]
    async fn reject_anonymous_refresh_and_profile() {
        let app = app_with_queries::<role::Model, _>(Vec::<Vec<role::Model>>::new());
        let (refresh_status, _) =
            request_method(app.clone(), "/sessions/refresh", axum::http::Method::POST).await;
        let (profile_status, _) = request(app, "/users/profile").await;
        assert_eq!(refresh_status, StatusCode::UNAUTHORIZED);
        assert_eq!(profile_status, StatusCode::UNAUTHORIZED);
    }
}
