pub mod common;
pub mod config;
pub mod entity;
pub mod error;
pub mod extract;
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
        features::session,
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

    async fn authorized_request(
        app: Router,
        path: &str,
        user_id: uuid::Uuid,
        role_id: uuid::Uuid,
    ) -> (StatusCode, Value) {
        let token =
            session::create_access(&config(), &user_id.to_string(), &role_id.to_string()).unwrap();
        let response = app
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
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
        let role_id = uuid::Uuid::new_v4();
        let model = user::Model {
            id,
            role_id,
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
        let role = role::Model {
            id: role_id,
            name: "customer".into(),
            created_at: now,
            updated_at: now,
        };
        let app = router(AppState {
            db: MockDatabase::new(DbBackend::Postgres)
                .append_query_results(vec![vec![model.clone()]])
                .append_query_results(vec![vec![role]])
                .append_query_results(vec![vec![model]])
                .into_connection(),
            redis: redis::Client::open(config().redis_url.clone()).unwrap(),
            storage: Storage::offline(&config()),
            config: config(),
        });
        let (status, body) = authorized_request(app, &format!("/users/{id}"), id, role_id).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["data"].get("password").is_none());
        assert_eq!(body["data"]["fullName"], "Dummy User");
    }

    #[tokio::test]
    async fn anonymous_cannot_read_another_users_address() {
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
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["statusCode"], 401);
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

    #[tokio::test]
    async fn collection_routes_accept_paths_without_trailing_slash() {
        let app = app_with_queries::<role::Model, _>(Vec::<Vec<role::Model>>::new());
        for path in ["/roles", "/users", "/addresses"] {
            let (status, _) = request(app.clone(), path).await;
            assert_ne!(
                status,
                StatusCode::NOT_FOUND,
                "collection route missing: {path}"
            );
        }
    }

    #[tokio::test]
    async fn invalid_json_and_query_parameters_use_json_error_envelopes() {
        let app = app_with_queries::<role::Model, _>(Vec::<Vec<role::Model>>::new());
        let invalid_body = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/roles")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":123}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_body.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(invalid_body.into_body(), 4096).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["statusCode"], 400);

        let (status, value) = request(app, "/roles?page=not-a-number").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["statusCode"], 400);
    }

    #[tokio::test]
    async fn mutations_without_authentication_are_rejected() {
        let app = app_with_queries::<role::Model, _>(Vec::<Vec<role::Model>>::new());
        let id = uuid::Uuid::new_v4();
        let update = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::PATCH)
                    .uri(format!("/users/{id}"))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"isActive":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update.status(), StatusCode::UNAUTHORIZED);

        let create_role = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/roles")
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"root"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_role.status(), StatusCode::UNAUTHORIZED);
    }
}
