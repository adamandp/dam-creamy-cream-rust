use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectOptions, ConnectionTrait, Database, DbBackend,
    EntityTrait, Statement,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    config::Config,
    entity::{inventory, order_item, product, product_information, role, user, user_voucher},
    features::session,
    infrastructure::storage::Storage,
    state::AppState,
};

async fn send(
    app: Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(v) => {
            request = request.header("Content-Type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn postgres_catalog_promotions_and_identity_interoperate() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    assert!(
        url.split('?')
            .next()
            .is_some_and(|u| u.ends_with("/creamy_cream_test")),
        "only test against disposable PostgreSQL"
    );
    let schema = format!("pr2_test_{}", Uuid::new_v4().simple());
    let admin_db = Database::connect(url.clone()).await.unwrap();
    admin_db
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("CREATE SCHEMA \"{schema}\""),
        ))
        .await
        .unwrap();
    let mut options = ConnectOptions::new(url);
    options.max_connections(1).min_connections(1);
    let db = Database::connect(options).await.unwrap();
    db.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        format!("SET search_path TO \"{schema}\""),
    ))
    .await
    .unwrap();
    for ddl in [
        "CREATE TYPE \"DISCOUNT_TYPE\" AS ENUM ('PERCENTAGE','FIXED','PRICE','FREE_ITEM')",
        "CREATE TYPE \"VOUCHER_TYPE\" AS ENUM ('PERCENTAGE','FIXED','FREE_ITEM')",
        "CREATE TYPE \"VOUCHER_GROUP\" AS ENUM ('CHECKOUT','SHIPPING')",
        "CREATE TYPE \"USER_VOUCHER_STATUS\" AS ENUM ('USED','CLAIMED')",
        "CREATE TYPE \"GENDER_TYPE\" AS ENUM ('MALE','FEMALE','OTHER')",
        "CREATE TABLE roles (id UUID PRIMARY KEY, name VARCHAR(50) NOT NULL, created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE users (id UUID PRIMARY KEY,role_id UUID NOT NULL REFERENCES roles(id),username VARCHAR(50) NOT NULL,email VARCHAR(100),phone_number VARCHAR(20),password VARCHAR(255) NOT NULL,full_name VARCHAR(100) NOT NULL,birth_date DATE,image_url VARCHAR(255),gender \"GENDER_TYPE\",is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE category (id UUID PRIMARY KEY,name VARCHAR(100) NOT NULL,description VARCHAR(255),is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE products (id UUID PRIMARY KEY,category_id UUID NOT NULL REFERENCES category(id),discount_id UUID,name VARCHAR(100) NOT NULL,image_url VARCHAR(255),description VARCHAR(255),price DOUBLE PRECISION NOT NULL,is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE inventory (id UUID PRIMARY KEY,product_id UUID NOT NULL UNIQUE REFERENCES products(id),quantity INTEGER NOT NULL,reserved_quantity INTEGER NOT NULL,is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE discounts (id UUID PRIMARY KEY,name VARCHAR(50) NOT NULL,discount_type \"DISCOUNT_TYPE\" NOT NULL,value DOUBLE PRECISION,max_applied INTEGER NOT NULL,max_redeemed INTEGER NOT NULL,redeemed_count INTEGER NOT NULL,description VARCHAR(255),product_id UUID,quantity INTEGER,start_date TIMESTAMP NOT NULL,end_date TIMESTAMP NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE product_discounts (id UUID PRIMARY KEY,product_id UUID NOT NULL UNIQUE REFERENCES products(id),discount_id UUID NOT NULL REFERENCES discounts(id),is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE reviews (id UUID PRIMARY KEY,user_id UUID NOT NULL REFERENCES users(id),product_id UUID NOT NULL REFERENCES products(id),rating INTEGER NOT NULL,comment VARCHAR(255),created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE order_items (id UUID PRIMARY KEY,product_id UUID NOT NULL REFERENCES products(id),quantity INTEGER NOT NULL)",
        "CREATE TABLE product_information (id UUID PRIMARY KEY,product_id UUID NOT NULL UNIQUE REFERENCES products(id),highlights JSONB NOT NULL,serving_suggestions JSONB NOT NULL,product_details JSONB NOT NULL,why_choose TEXT NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE vouchers (id UUID PRIMARY KEY,code VARCHAR(50) NOT NULL,name VARCHAR(50) NOT NULL,description VARCHAR(255),type \"VOUCHER_TYPE\" NOT NULL,\"group\" \"VOUCHER_GROUP\" NOT NULL,value DOUBLE PRECISION,product_id UUID REFERENCES products(id),quantity INTEGER,minimum_purchase DOUBLE PRECISION NOT NULL,is_active BOOLEAN NOT NULL,start_date TIMESTAMP NOT NULL,end_date TIMESTAMP NOT NULL,max_redeemed INTEGER NOT NULL,redeemed_count INTEGER NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE user_vouchers (id UUID PRIMARY KEY,user_id UUID NOT NULL REFERENCES users(id),voucher_id UUID NOT NULL REFERENCES vouchers(id),status \"USER_VOUCHER_STATUS\" NOT NULL,claimed_at TIMESTAMP NOT NULL,expired_at TIMESTAMP NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
    ] {
        db.execute_raw(Statement::from_string(DbBackend::Postgres, ddl.to_owned()))
            .await
            .unwrap();
    }
    let now = Utc::now().naive_utc();
    let role_id = Uuid::new_v4();
    role::ActiveModel {
        id: Set(role_id),
        name: Set("super-admin".into()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    let user_id = Uuid::new_v4();
    user::ActiveModel {
        id: Set(user_id),
        role_id: Set(role_id),
        username: Set("catalogtester".into()),
        email: Set(None),
        phone_number: Set(None),
        password: Set("password-placeholder".into()),
        full_name: Set("Catalog Tester".into()),
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
    let config = Config {
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
    };
    let token =
        session::create_access(&config, &user_id.to_string(), &role_id.to_string()).unwrap();
    let app = crate::router(AppState {
        db: db.clone(),
        redis: redis::Client::open(config.redis_url.clone()).unwrap(),
        storage: Storage::offline(&config),
        config,
    });
    let authorized = Some(token.as_str());
    let (status, _) = send(
        app.clone(),
        Method::POST,
        "/categories",
        None,
        Some(json!({"name":"Ice Cream","description":"Creamy","isActive":true})),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        app.clone(),
        Method::POST,
        "/categories",
        authorized,
        Some(json!({"name":"Ice Cream","description":"Creamy","isActive":true})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cat = crate::entity::category::Entity::find()
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let (status,_)=send(app.clone(),Method::POST,"/products",authorized,Some(json!({"categoryId":cat.id,"name":"Creamy Vanilla","description":"Milk based","price":10_000.0,"isActive":true}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let product = product::Entity::find().one(&db).await.unwrap().unwrap();
    inventory::ActiveModel {
        id: Set(Uuid::new_v4()),
        product_id: Set(product.id),
        quantity: Set(20),
        reserved_quantity: Set(0),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    order_item::ActiveModel {
        id: Set(Uuid::new_v4()),
        product_id: Set(product.id),
        quantity: Set(3),
    }
    .insert(&db)
    .await
    .unwrap();
    product_information::ActiveModel {
        id: Set(Uuid::new_v4()),
        product_id: Set(product.id),
        highlights: Set(json!(["Fresh"])),
        serving_suggestions: Set(json!(["Cold"])),
        product_details: Set(json!({"size":"1 L","storage":"Freezer","shelfLife":"1 year"})),
        why_choose: Set("Delicious".into()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    let (status,_)=send(app.clone(),Method::POST,"/discounts",authorized,Some(json!({"name":"Ten Percent","discountType":"PERCENTAGE","value":10.0,"maxApplied":1,"maxRedeemed":100,"redeemedCount":0,"description":null,"productId":product.id,"quantity":null,"startDate":"2026-01-01T00:00:00Z","endDate":"2027-01-01T00:00:00Z"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let discount = crate::entity::discount::Entity::find()
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let (status, _) = send(
        app.clone(),
        Method::POST,
        "/product-discounts",
        authorized,
        Some(json!({"productId":product.id,"discountId":discount.id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, catalog) = send(
        app.clone(),
        Method::GET,
        "/products/catalog?q=cream&min=1000&sort=asc",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(catalog["data"][0]["discountPrice"], 9000.0);
    assert_eq!(catalog["data"][0]["discountType"], "PERCENTAGE");
    let (status, details) = send(
        app.clone(),
        Method::GET,
        &format!("/product/{}", product.id),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(details["data"]["sales"], 3);
    assert_eq!(details["data"]["category"], "Ice Cream");
    let (status, info) = send(
        app.clone(),
        Method::GET,
        &format!("/product/information/{}", product.id),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        info["data"]["information"]
            .as_str()
            .unwrap()
            .contains("Shelf life: 1 year")
    );
    let (status, _) = send(
        app.clone(),
        Method::POST,
        "/reviews",
        authorized,
        Some(json!({"userId":user_id,"productId":product.id,"rating":5,"comment":"Nice"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, reviews) = send(
        app.clone(),
        Method::GET,
        &format!("/reviews/product/{}", product.id),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reviews["data"][0]["rate"], 5);
    let inv = inventory::Entity::find().one(&db).await.unwrap().unwrap();
    let (status, _) = send(
        app.clone(),
        Method::PATCH,
        &format!("/inventory/{}", inv.id),
        authorized,
        Some(json!({"reservedQuantity":21})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(
        app.clone(),
        Method::PATCH,
        &format!("/inventory/{}", inv.id),
        authorized,
        Some(json!({"quantity":30})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status,_)=send(app.clone(),Method::POST,"/vouchers",authorized,Some(json!({"code":"WELCOME","name":"Welcome","description":null,"type":"FIXED","group":"CHECKOUT","value":1000.0,"productId":product.id,"quantity":null,"minimumPurchase":5000.0,"isActive":true,"startDate":"2026-01-01T00:00:00Z","endDate":"2027-01-01T00:00:00Z","maxRedeemed":100,"redeemedCount":0}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let voucher = crate::entity::voucher::Entity::find()
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let (status,_)=send(app.clone(),Method::POST,"/user-vouchers",authorized,Some(json!({"userId":user_id,"voucherId":voucher.id,"claimedAt":"2026-11-01T00:00:00Z","expiredAt":"2026-12-01T00:00:00Z"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, mine) = send(
        app.clone(),
        Method::GET,
        "/user-vouchers/user",
        authorized,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine["data"][0]["vouchers"]["code"], "WELCOME");
    let uv = user_voucher::Entity::find()
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let (status, _) = send(
        app.clone(),
        Method::PATCH,
        &format!("/user-vouchers/{}", uv.id),
        authorized,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        app,
        Method::GET,
        &format!("/user-vouchers/{}", uv.id),
        authorized,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["status"], "USED");
    drop(db);
    admin_db
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("DROP SCHEMA \"{schema}\" CASCADE"),
        ))
        .await
        .unwrap();
}
