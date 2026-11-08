pub mod common;
pub mod config;
pub mod entity;
pub mod error;
pub mod extract;
pub mod features;
pub mod infrastructure;
pub mod state;

#[cfg(test)]
mod pr2_tests;

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
        .nest("/categories", features::categories::routes())
        .nest("/products", features::products::routes())
        .nest("/product", features::product::routes())
        .nest("/inventory", features::inventory::routes())
        .nest("/reviews", features::reviews::routes())
        .nest("/discounts", features::discounts::routes())
        .nest("/product-discounts", features::product_discounts::routes())
        .nest("/vouchers", features::vouchers::routes())
        .nest("/user-vouchers", features::user_vouchers::routes())
        .route("/media/users/{id}/{asset}", get(features::users::get_image))
        .route(
            "/media/products/{id}/{asset}",
            get(features::products::get_image),
        )
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
    use sea_orm::{
        ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, Database, DbBackend,
        EntityTrait, MockDatabase, QueryFilter, QuerySelect, Statement, TransactionTrait,
    };
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::{
        config::Config,
        entity::{address, cart, role, user, user_token},
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
    async fn logout_clears_revoked_refresh_cookies_even_on_unauthorized() {
        let app = app_with_queries::<role::Model, _>(Vec::<Vec<role::Model>>::new());
        for path in ["/auth/logout", "/auth/logout-all"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(axum::http::Method::POST)
                        .uri(path)
                        .header(axum::http::header::COOKIE, "refreshToken=invalid")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            let cookie = response
                .headers()
                .get(axum::http::header::SET_COOKIE)
                .expect("logout must expire revoked refresh cookies")
                .to_str()
                .unwrap();
            assert!(cookie.contains("refreshToken="), "{cookie}");
            assert!(cookie.contains("Max-Age=0"), "{cookie}");
        }
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

    async fn send_json(
        app: Router,
        method: axum::http::Method,
        path: String,
        token: String,
        body: Value,
    ) -> StatusCode {
        app.oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    // Runs against the isolated PostgreSQL service in CI. Does not create or
    // migrate tables in any application database.
    #[tokio::test]
    async fn postgres_authorization_and_primary_address_invariants() {
        let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
            return;
        };
        assert!(
            url.split('?')
                .next()
                .is_some_and(|value| value.ends_with("/creamy_cream_test")),
            "integration test may only touch the disposable creamy_cream_test database"
        );
        let db = Database::connect(url).await.unwrap();
        for ddl in [
            "CREATE TYPE \"GENDER_TYPE\" AS ENUM ('MALE', 'FEMALE', 'OTHER')",
            "CREATE TABLE roles (id UUID PRIMARY KEY, name VARCHAR(50) UNIQUE NOT NULL, created_at TIMESTAMP NOT NULL, updated_at TIMESTAMP NOT NULL)",
            "CREATE TABLE users (id UUID PRIMARY KEY, role_id UUID NOT NULL REFERENCES roles(id), username VARCHAR(50) UNIQUE NOT NULL, email VARCHAR(255), phone_number VARCHAR(100), password VARCHAR(255) NOT NULL, full_name VARCHAR(100) NOT NULL, birth_date DATE, image_url VARCHAR(255), gender \"GENDER_TYPE\", is_active BOOLEAN NOT NULL, created_at TIMESTAMP NOT NULL, updated_at TIMESTAMP NOT NULL)",
            "CREATE TABLE addresses (id UUID PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id), recipient_name VARCHAR(100) NOT NULL, recipient_phone_number VARCHAR(100) NOT NULL, province VARCHAR(100) NOT NULL, city VARCHAR(100) NOT NULL, subdistrict VARCHAR(100) NOT NULL, village VARCHAR(100) NOT NULL, postal_code VARCHAR(50) NOT NULL, address VARCHAR(255) NOT NULL, is_primary BOOLEAN NOT NULL, created_at TIMESTAMP NOT NULL, updated_at TIMESTAMP NOT NULL)",
            "CREATE TABLE carts (id UUID PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id), created_at TIMESTAMP NOT NULL, updated_at TIMESTAMP NOT NULL)",
            "CREATE TABLE cart_items (id UUID PRIMARY KEY, cart_id UUID NOT NULL REFERENCES carts(id))",
            "CREATE TABLE user_tokens (id UUID PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id), token VARCHAR(255) UNIQUE NOT NULL, \"browserInfo\" TEXT NOT NULL, \"expiredAt\" TIMESTAMP NOT NULL, created_at TIMESTAMP NOT NULL, updated_at TIMESTAMP NOT NULL)",
            "CREATE TABLE orders (id UUID PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id), address_id UUID NOT NULL REFERENCES addresses(id))",
            "CREATE TABLE reviews (id UUID PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id))",
            "CREATE TABLE user_vouchers (id UUID PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id))",
            "CREATE TABLE product_analytics (id UUID PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id))",
            "CREATE TABLE user_notifications (id UUID PRIMARY KEY, user_id UUID NOT NULL REFERENCES users(id))",
        ] {
            db.execute_raw(Statement::from_string(DbBackend::Postgres, ddl.to_owned()))
                .await
                .unwrap();
        }
        let now = Utc::now().naive_utc();
        let customer_role = uuid::Uuid::new_v4();
        let admin_role = uuid::Uuid::new_v4();
        let root_role = uuid::Uuid::new_v4();
        for (id, name) in [
            (customer_role, "customer"),
            (admin_role, "admin"),
            (root_role, "super-admin"),
        ] {
            role::ActiveModel {
                id: Set(id),
                name: Set(name.to_owned()),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&db)
            .await
            .unwrap();
        }
        let owner = uuid::Uuid::new_v4();
        let stranger = uuid::Uuid::new_v4();
        let administrator = uuid::Uuid::new_v4();
        let root = uuid::Uuid::new_v4();
        for (id, role_id, username) in [
            (owner, customer_role, "integrationowner"),
            (stranger, customer_role, "integrationstranger"),
            (administrator, admin_role, "integrationadmin"),
            (root, root_role, "integrationroot"),
        ] {
            user::ActiveModel {
                id: Set(id),
                role_id: Set(role_id),
                username: Set(username.to_owned()),
                email: Set(None),
                phone_number: Set(None),
                password: Set("unused-hash".to_owned()),
                full_name: Set("Test User".to_owned()),
                birth_date: Set(None),
                image_url: Set(None),
                gender: Set(None),
                is_active: Set(true),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&db)
            .await
            .unwrap();
        }
        let cfg = config();
        let token_for = |id: uuid::Uuid, role_id: uuid::Uuid| {
            session::create_access(&cfg, &id.to_string(), &role_id.to_string()).unwrap()
        };
        let owner_token = token_for(owner, customer_role);
        let stranger_token = token_for(stranger, customer_role);
        let admin_token = token_for(administrator, admin_role);
        let root_token = token_for(root, root_role);
        let app = router(AppState {
            db: db.clone(),
            redis: redis::Client::open(cfg.redis_url.clone()).unwrap(),
            storage: Storage::offline(&cfg),
            config: cfg,
        });

        // The fixture assigns identical created_at timestamps to every role
        // and user. This catches offset pagination without a deterministic
        // UUID tie-breaker, including across consecutive page requests.
        let mut role_ids = [customer_role, admin_role, root_role];
        role_ids.sort_unstable();
        for (index, expected_id) in role_ids.iter().enumerate() {
            let (status, result) =
                request(app.clone(), &format!("/roles?page={}&limit=1", index + 1)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(result["data"][0]["id"], expected_id.to_string());
        }
        let mut user_ids = [owner, stranger, administrator, root];
        user_ids.sort_unstable();
        for (index, expected_id) in user_ids.iter().enumerate() {
            let (status, result) = authorized_request(
                app.clone(),
                &format!("/users?page={}&limit=1", index + 1),
                root,
                root_role,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(result["data"][0]["id"], expected_id.to_string());
        }

        // Legacy validateUpdate rejects an empty patch as well as fields
        // explicitly submitted with unchanged values.
        for patch in [json!({}), json!({"fullName": "Test User"})] {
            assert_eq!(
                send_json(
                    app.clone(),
                    axum::http::Method::PATCH,
                    format!("/users/{owner}"),
                    owner_token.clone(),
                    patch,
                )
                .await,
                StatusCode::BAD_REQUEST,
            );
        }

        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/users/{owner}"),
                stranger_token.clone(),
                json!({"roleId": admin_role})
            )
            .await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/users/{owner}"),
                owner_token.clone(),
                json!({"roleId": admin_role})
            )
            .await,
            StatusCode::FORBIDDEN
        );
        for (target, body) in [
            (administrator, json!({"roleId": root_role})),
            (root, json!({"isActive": false})),
            (root, json!({"fullName": "Compromised"})),
            (owner, json!({"roleId": root_role})),
        ] {
            assert_eq!(
                send_json(
                    app.clone(),
                    axum::http::Method::PATCH,
                    format!("/users/{target}"),
                    admin_token.clone(),
                    body,
                )
                .await,
                StatusCode::FORBIDDEN,
            );
        }
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::DELETE,
                format!("/users/{root}"),
                admin_token.clone(),
                json!({}),
            )
            .await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::DELETE,
                format!("/users/{root}"),
                root_token.clone(),
                json!({}),
            )
            .await,
            StatusCode::FORBIDDEN,
            "super-admin must not be able to delete the last admin account",
        );
        assert!(
            user::Entity::find_by_id(root)
                .one(&db)
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/users/{owner}"),
                owner_token.clone(),
                json!({"password": "Password123!"})
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::POST,
                "/roles".into(),
                admin_token.clone(),
                json!({"name": "privileged"})
            )
            .await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/roles/{root_role}"),
                root_token.clone(),
                json!({"name": "owner"})
            )
            .await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::DELETE,
                format!("/roles/{root_role}"),
                root_token.clone(),
                json!({})
            )
            .await,
            StatusCode::FORBIDDEN
        );

        let address_body = |recipient: &str, primary: bool| {
            json!({
                "userId": owner,
                "recipientName": recipient,
                "recipientPhoneNumber": "081234567890",
                "province": "Banten",
                "city": "Tangerang",
                "subdistrict": "Serpong",
                "village": "Lengkong",
                "postalCode": "15310",
                "address": "Street One",
                "isPrimary": primary
            })
        };
        let mut invalid_postal_code = address_body("Invalid postal", false);
        invalid_postal_code["postalCode"] = json!("X".repeat(101));
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::POST,
                "/addresses".into(),
                owner_token.clone(),
                invalid_postal_code
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::POST,
                "/addresses".into(),
                owner_token.clone(),
                address_body("First", false)
            )
            .await,
            StatusCode::CREATED
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::POST,
                "/addresses".into(),
                owner_token.clone(),
                address_body("Second", true)
            )
            .await,
            StatusCode::CREATED
        );
        let rows = address::Entity::find()
            .filter(address::Column::UserId.eq(owner))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(rows.iter().filter(|address| address.is_primary).count(), 1);

        let mut ordered_addresses = rows.clone();
        ordered_addresses.sort_by_key(|row| (row.created_at, row.id));
        for (index, expected) in ordered_addresses.iter().enumerate() {
            let (status, result) = authorized_request(
                app.clone(),
                &format!("/addresses?page={}&limit=1", index + 1),
                root,
                root_role,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(result["data"][0]["id"], expected.id.to_string());
        }

        let first = rows
            .iter()
            .find(|address| address.recipient_name == "First")
            .unwrap()
            .id;
        let second = rows
            .iter()
            .find(|address| address.recipient_name == "Second")
            .unwrap()
            .id;
        for patch in [json!({}), json!({"city": "Tangerang"})] {
            assert_eq!(
                send_json(
                    app.clone(),
                    axum::http::Method::PATCH,
                    format!("/addresses/{first}"),
                    owner_token.clone(),
                    patch,
                )
                .await,
                StatusCode::BAD_REQUEST,
            );
        }
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/addresses/{first}"),
                owner_token.clone(),
                json!({"postalCode": "X".repeat(101)})
            )
            .await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/addresses/{first}"),
                owner_token.clone(),
                json!({"isPrimary": true})
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/addresses/{first}"),
                owner_token.clone(),
                json!({"isPrimary": false})
            )
            .await,
            StatusCode::OK
        );
        let second_row = address::Entity::find_by_id(second)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(second_row.is_primary);
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/addresses/{first}"),
                stranger_token.clone(),
                json!({"city": "Bandung"})
            )
            .await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::POST,
                "/addresses".into(),
                stranger_token.clone(),
                address_body("Third", true)
            )
            .await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/addresses/{first}"),
                admin_token.clone(),
                json!({"userId": stranger})
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            address::Entity::find_by_id(first)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .user_id,
            stranger
        );
        assert!(
            address::Entity::find_by_id(first)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .is_primary
        );
        assert!(
            address::Entity::find_by_id(second)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .is_primary
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::DELETE,
                format!("/addresses/{first}"),
                stranger_token.clone(),
                json!({})
            )
            .await,
            StatusCode::CONFLICT
        );

        let mut simultaneous = Vec::new();
        for _ in 0..6 {
            simultaneous.push(tokio::spawn(send_json(
                app.clone(),
                axum::http::Method::POST,
                "/addresses".into(),
                owner_token.clone(),
                address_body("Extra", true),
            )));
        }
        for request in simultaneous {
            assert_eq!(request.await.unwrap(), StatusCode::CREATED);
        }
        let rows = address::Entity::find()
            .filter(address::Column::UserId.eq(owner))
            .all(&db)
            .await
            .unwrap();
        assert_eq!(rows.iter().filter(|address| address.is_primary).count(), 1);
        // An order address must never be assigned to a different user.
        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO orders (id, user_id, address_id) VALUES ($1, $2, $3)",
            [uuid::Uuid::new_v4().into(), owner.into(), second.into()],
        ))
        .await
        .unwrap();
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/addresses/{second}"),
                admin_token.clone(),
                json!({"userId": stranger})
            )
            .await,
            StatusCode::CONFLICT
        );
        assert_eq!(
            address::Entity::find_by_id(second)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .user_id,
            owner
        );
        // The actual schema prevents deleting accounts that have any FK
        // dependencies. Deletion must clean an empty cart and stale sessions.
        let empty_cart = uuid::Uuid::new_v4();
        cart::ActiveModel {
            id: Set(empty_cart),
            user_id: Set(stranger),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .unwrap();
        db.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO cart_items (id, cart_id) VALUES ($1, $2)",
            [uuid::Uuid::new_v4().into(), empty_cart.into()],
        ))
        .await
        .unwrap();
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::DELETE,
                format!("/users/{stranger}"),
                stranger_token,
                json!({}),
            )
            .await,
            StatusCode::OK
        );
        assert!(
            user::Entity::find_by_id(stranger)
                .one(&db)
                .await
                .unwrap()
                .is_none()
        );

        // A user with an order must remain intact, including address history.
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::DELETE,
                format!("/users/{owner}"),
                owner_token.clone(),
                json!({}),
            )
            .await,
            StatusCode::CONFLICT
        );
        assert!(
            user::Entity::find_by_id(owner)
                .one(&db)
                .await
                .unwrap()
                .is_some()
        );

        // Deactivated accounts cannot use either still-valid access or
        // stored refresh tokens; dummy behavior remains development-only.
        let inactive = uuid::Uuid::new_v4();
        user::ActiveModel {
            id: Set(inactive),
            role_id: Set(customer_role),
            username: Set("integrationinactive".into()),
            email: Set(None),
            phone_number: Set(None),
            password: Set("unused-hash".into()),
            full_name: Set("Inactive User".into()),
            birth_date: Set(None),
            image_url: Set(None),
            gender: Set(None),
            is_active: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .unwrap();
        let runtime = AppState {
            db: db.clone(),
            redis: redis::Client::open(config().redis_url.clone()).unwrap(),
            storage: Storage::offline(&config()),
            config: config(),
        };
        let (_, refresh_token) = session::create_tokens(
            &runtime,
            &inactive.to_string(),
            &customer_role.to_string(),
            None,
            "test-device".into(),
        )
        .await
        .unwrap();
        // Simulate an external/legacy service deactivating a user without
        // revoking refresh tokens. /addresses/user must independently block it.
        let mut externally_disabled: user::ActiveModel = user::Entity::find_by_id(inactive)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
        externally_disabled.is_active = Set(false);
        externally_disabled.update(&db).await.unwrap();
        let address_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/addresses/user")
                    .header(
                        axum::http::header::COOKIE,
                        format!("refreshToken={refresh_token}"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(address_response.status(), StatusCode::UNAUTHORIZED);
        let mut enabled_again: user::ActiveModel = user::Entity::find_by_id(inactive)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
        enabled_again.is_active = Set(true);
        enabled_again.update(&db).await.unwrap();
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/users/{inactive}"),
                admin_token.clone(),
                json!({"isActive": false}),
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            authorized_request(app.clone(), "/users/profile", inactive, customer_role)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/sessions/refresh")
                    .header(
                        axum::http::header::COOKIE,
                        format!("refreshToken={refresh_token}"),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            user_token::Entity::find()
                .filter(user_token::Column::UserId.eq(inactive))
                .all(&db)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PATCH,
                format!("/users/{inactive}"),
                admin_token.clone(),
                json!({"isActive": true}),
            )
            .await,
            StatusCode::OK
        );
        assert!(matches!(
            session::valid_stored_refresh(&runtime, &refresh_token).await,
            Err(crate::error::AppError::Unauthorized(_))
        ));

        // A password update atomically revokes every existing refresh session.
        let mut password_user: user::ActiveModel = user::Entity::find_by_id(root)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .into();
        password_user.password = Set(crate::features::auth::hash_password("OldPassword1!".into())
            .await
            .unwrap());
        password_user.update(&db).await.unwrap();
        let (_, old_refresh) = session::create_tokens(
            &runtime,
            &root.to_string(),
            &root_role.to_string(),
            None,
            "test-device".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            send_json(
                app.clone(),
                axum::http::Method::PUT,
                format!("/users/{root}/password"),
                root_token,
                json!({"exsitingPassword": "OldPassword1!", "newPassword": "NewPassword2!"}),
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            user_token::Entity::find()
                .filter(user_token::Column::UserId.eq(root))
                .all(&db)
                .await
                .unwrap()
                .len(),
            0
        );
        assert!(matches!(
            session::valid_stored_refresh(&runtime, &old_refresh).await,
            Err(crate::error::AppError::Unauthorized(_))
        ));

        // An obsolete-image cleanup must never remove a key that became the
        // currently referenced image. Offline storage deliberately fails on
        // deletion, so Ok(()) proves that the key was protected by the row
        // lock and active-reference check, without requiring live MinIO.
        let asset = uuid::Uuid::new_v4();
        let active_key = format!("users/{root}/{asset}");
        let tx = db.begin().await.unwrap();
        let current = user::Entity::find_by_id(root)
            .lock_exclusive()
            .one(&tx)
            .await
            .unwrap()
            .unwrap();
        let mut active: user::ActiveModel = current.into();
        active.image_url = Set(Some(format!(
            "{}/media/users/{root}/{asset}",
            runtime.config.public_base_url
        )));
        active.update(&tx).await.unwrap();
        let pending_cleanup = tokio::spawn({
            let state = runtime.clone();
            let key = active_key.clone();
            async move { crate::features::users::delete_image_if_unreferenced(&state, root, &key).await }
        });
        tokio::task::yield_now().await;
        tx.commit().await.unwrap();
        pending_cleanup.await.unwrap().unwrap();
        assert_eq!(
            user::Entity::find_by_id(root)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .image_url,
            Some(format!(
                "{}/media/users/{root}/{asset}",
                runtime.config.public_base_url
            ))
        );
    }
}
