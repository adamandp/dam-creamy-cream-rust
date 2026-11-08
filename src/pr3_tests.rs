use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use chrono::{Duration, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectOptions, ConnectionTrait, Database, DbBackend,
    EntityTrait, Statement,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::{
    config::Config,
    entity::{
        address, cart, cart_item, category, discount, inventory, order, order_item, order_voucher,
        payment, product, product_discount, role, shipment, user, user_voucher, voucher,
    },
    features::{
        orders::{CheckoutInput, checkout},
        session,
    },
    infrastructure::storage::Storage,
    state::AppState,
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

async fn send(
    app: axum::Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let body = if let Some(body) = body {
        builder = builder.header("Content-Type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = app.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[test]
fn checkout_schema_is_compatible_with_nest_dto() {
    let valid = json!({"order":{"userId":Uuid::new_v4(),"addressId":Uuid::new_v4(),"voucherIds":null},
        "orderItems":[{"productId":Uuid::new_v4(),"discountId":null,"quantity":2}],
        "shipment":{"courier":"jne","service":"REG","cost":12000,"estimated":"2026-11-11","description":null},
        "payment":{"method":"BCA"},"ignoredByZod":true});
    let checkout: CheckoutInput = serde_json::from_value(valid.clone()).unwrap();
    assert!(checkout.validate().is_ok());
    for missing in ["description", "courier", "service", "estimated", "cost"] {
        let mut no_field = valid.clone();
        no_field["shipment"]
            .as_object_mut()
            .unwrap()
            .remove(missing);
        assert!(
            serde_json::from_value::<CheckoutInput>(no_field).is_err(),
            "{missing} is required"
        );
    }
    let mut no_items = valid.clone();
    no_items["orderItems"] = json!([]);
    assert!(
        serde_json::from_value::<CheckoutInput>(no_items)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut too_many_vouchers = valid;
    too_many_vouchers["order"]["voucherIds"] =
        json!([Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()]);
    assert!(
        serde_json::from_value::<CheckoutInput>(too_many_vouchers)
            .unwrap()
            .validate()
            .is_err()
    );
}

#[tokio::test]
async fn postgres_checkout_rolls_back_late_failure_then_commits_related_rows() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    assert!(
        url.split('?')
            .next()
            .is_some_and(|v| v.ends_with("/creamy_cream_test")),
        "only use disposable creamy_cream_test PostgreSQL"
    );
    let schema = format!("pr3_test_{}", Uuid::new_v4().simple());
    let root = Database::connect(url.clone()).await.unwrap();
    root.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        format!("CREATE SCHEMA \"{schema}\""),
    ))
    .await
    .unwrap();
    let mut options = ConnectOptions::new(url.clone());
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
        "CREATE TYPE \"ORDER_STATUS\" AS ENUM ('PENDING','PAID','SHIPPED','DELIVERED','CANCELLED')",
        "CREATE TYPE \"PAYMENT_STATUS\" AS ENUM ('PENDING','PAID','FAILED','CANCELLED')",
        "CREATE TYPE \"SHIPMENT_STATUS\" AS ENUM ('PENDING','SHIPPED','DELIVERED','CANCELLED')",
        "CREATE TABLE roles (id UUID PRIMARY KEY,name VARCHAR(50) NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE users (id UUID PRIMARY KEY,role_id UUID NOT NULL REFERENCES roles(id),username VARCHAR(50) NOT NULL,email VARCHAR(255),phone_number VARCHAR(100),password VARCHAR(255) NOT NULL,full_name VARCHAR(100) NOT NULL,birth_date DATE,image_url VARCHAR(255),gender \"GENDER_TYPE\",is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE addresses (id UUID PRIMARY KEY,user_id UUID NOT NULL REFERENCES users(id),recipient_name VARCHAR(100) NOT NULL,recipient_phone_number VARCHAR(100) NOT NULL,province VARCHAR(100) NOT NULL,city VARCHAR(100) NOT NULL,subdistrict VARCHAR(100) NOT NULL,village VARCHAR(100) NOT NULL,postal_code VARCHAR(50) NOT NULL,address VARCHAR(255) NOT NULL,is_primary BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE category (id UUID PRIMARY KEY,name VARCHAR(100) NOT NULL,description VARCHAR(255),is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE products (id UUID PRIMARY KEY,category_id UUID NOT NULL REFERENCES category(id),discount_id UUID,name VARCHAR(100) NOT NULL,image_url VARCHAR(255),description VARCHAR(255),price DOUBLE PRECISION NOT NULL,is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE inventory (id UUID PRIMARY KEY,product_id UUID NOT NULL REFERENCES products(id),quantity INTEGER NOT NULL,reserved_quantity INTEGER NOT NULL,is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE carts (id UUID PRIMARY KEY,user_id UUID NOT NULL REFERENCES users(id),created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE cart_items (id UUID PRIMARY KEY,cart_id UUID NOT NULL REFERENCES carts(id),product_id UUID NOT NULL REFERENCES products(id),quantity INTEGER NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL,UNIQUE(cart_id,product_id))",
        "CREATE TABLE discounts (id UUID PRIMARY KEY,name VARCHAR(50) NOT NULL,discount_type \"DISCOUNT_TYPE\" NOT NULL,value DOUBLE PRECISION,max_applied INTEGER NOT NULL,max_redeemed INTEGER NOT NULL,redeemed_count INTEGER NOT NULL,description VARCHAR(255),product_id UUID,quantity INTEGER,start_date TIMESTAMP NOT NULL,end_date TIMESTAMP NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE product_discounts (id UUID PRIMARY KEY,product_id UUID NOT NULL REFERENCES products(id),discount_id UUID NOT NULL REFERENCES discounts(id),is_active BOOLEAN NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE vouchers (id UUID PRIMARY KEY,code VARCHAR(50) NOT NULL,name VARCHAR(50) NOT NULL,description VARCHAR(255),type \"VOUCHER_TYPE\" NOT NULL,\"group\" \"VOUCHER_GROUP\" NOT NULL,value DOUBLE PRECISION,product_id UUID,quantity INTEGER,minimum_purchase DOUBLE PRECISION NOT NULL,is_active BOOLEAN NOT NULL,start_date TIMESTAMP NOT NULL,end_date TIMESTAMP NOT NULL,max_redeemed INTEGER NOT NULL,redeemed_count INTEGER NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE user_vouchers (id UUID PRIMARY KEY,user_id UUID NOT NULL REFERENCES users(id),voucher_id UUID NOT NULL REFERENCES vouchers(id),status \"USER_VOUCHER_STATUS\" NOT NULL,claimed_at TIMESTAMP NOT NULL,expired_at TIMESTAMP NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE orders (id UUID PRIMARY KEY,user_id UUID NOT NULL REFERENCES users(id),address_id UUID NOT NULL REFERENCES addresses(id),voucher_amount DOUBLE PRECISION NOT NULL,discount_amount DOUBLE PRECISION NOT NULL,total_price DOUBLE PRECISION NOT NULL,final_price DOUBLE PRECISION NOT NULL,status \"ORDER_STATUS\" NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE order_items (id UUID PRIMARY KEY,order_id UUID NOT NULL REFERENCES orders(id),product_id UUID NOT NULL REFERENCES products(id),discount_id UUID,discount_amount DOUBLE PRECISION NOT NULL,price DOUBLE PRECISION NOT NULL,\"finalPrice\" DOUBLE PRECISION NOT NULL,quantity INTEGER NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE order_vouchers (order_id UUID NOT NULL REFERENCES orders(id),voucher_id UUID NOT NULL REFERENCES vouchers(id),PRIMARY KEY(order_id,voucher_id))",
        "CREATE TABLE payments (id UUID PRIMARY KEY,order_id UUID NOT NULL REFERENCES orders(id),method VARCHAR(50) NOT NULL,method_type VARCHAR(50) NOT NULL,amount DOUBLE PRECISION NOT NULL,status \"PAYMENT_STATUS\" NOT NULL,transaction_code VARCHAR(255),transaction_id VARCHAR(255),expiry_date TIMESTAMP,paid_at TIMESTAMP,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
        "CREATE TABLE shipments (id UUID PRIMARY KEY,order_id UUID NOT NULL REFERENCES orders(id),courier VARCHAR(50) NOT NULL,service VARCHAR(50) NOT NULL,cost DOUBLE PRECISION NOT NULL,estimated TIMESTAMP NOT NULL,delivered_at TIMESTAMP,description VARCHAR(255),tracking_number VARCHAR(50),status \"SHIPMENT_STATUS\" NOT NULL,created_at TIMESTAMP NOT NULL,updated_at TIMESTAMP NOT NULL)",
    ] {
        db.execute_raw(Statement::from_string(DbBackend::Postgres, ddl.to_owned()))
            .await
            .unwrap();
    }

    let now = Utc::now().naive_utc();
    let role_id = Uuid::new_v4();
    role::ActiveModel {
        id: Set(role_id),
        name: Set("customer".into()),
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
        username: Set("commerce_tester".into()),
        email: Set(None),
        phone_number: Set(None),
        password: Set("hash".into()),
        full_name: Set("Commerce Tester".into()),
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
    let address_id = Uuid::new_v4();
    address::ActiveModel {
        id: Set(address_id),
        user_id: Set(user_id),
        recipient_name: Set("Adam".into()),
        recipient_phone_number: Set("0800".into()),
        province: Set("Jawa Barat".into()),
        city: Set("Bandung".into()),
        subdistrict: Set("Coblong".into()),
        village: Set("Dago".into()),
        postal_code: Set("40135".into()),
        address: Set("Jl Test".into()),
        is_primary: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    let category_id = Uuid::new_v4();
    category::ActiveModel {
        id: Set(category_id),
        name: Set("Ice Cream".into()),
        description: Set(None),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    let product_id = Uuid::new_v4();
    product::ActiveModel {
        id: Set(product_id),
        category_id: Set(category_id),
        discount_id: Set(None),
        name: Set("Vanilla".into()),
        image_url: Set(None),
        description: Set(None),
        price: Set(100.0),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    inventory::ActiveModel {
        id: Set(Uuid::new_v4()),
        product_id: Set(product_id),
        quantity: Set(4),
        reserved_quantity: Set(0),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    let cart_id = Uuid::new_v4();
    cart::ActiveModel {
        id: Set(cart_id),
        user_id: Set(user_id),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    cart_item::ActiveModel {
        id: Set(Uuid::new_v4()),
        cart_id: Set(cart_id),
        product_id: Set(product_id),
        quantity: Set(5),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    let discount_id = Uuid::new_v4();
    discount::ActiveModel {
        id: Set(discount_id),
        name: Set("Ten Off".into()),
        discount_type: Set(discount::DiscountType::Fixed),
        value: Set(Some(10.0)),
        max_applied: Set(2),
        max_redeemed: Set(2),
        redeemed_count: Set(0),
        description: Set(None),
        product_id: Set(Some(product_id)),
        quantity: Set(None),
        start_date: Set(now - Duration::days(1)),
        end_date: Set(now + Duration::days(1)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    product_discount::ActiveModel {
        id: Set(Uuid::new_v4()),
        product_id: Set(product_id),
        discount_id: Set(discount_id),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    let voucher_id = Uuid::new_v4();
    voucher::ActiveModel {
        id: Set(voucher_id),
        code: Set("SHIP5".into()),
        name: Set("Shipping".into()),
        description: Set(None),
        r#type: Set(voucher::VoucherType::Fixed),
        group: Set(voucher::VoucherGroup::Shipping),
        value: Set(Some(5.0)),
        product_id: Set(None),
        quantity: Set(None),
        minimum_purchase: Set(0.0),
        is_active: Set(true),
        start_date: Set(now - Duration::days(1)),
        end_date: Set(now + Duration::days(1)),
        max_redeemed: Set(10),
        redeemed_count: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    user_voucher::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(user_id),
        voucher_id: Set(voucher_id),
        status: Set(user_voucher::UserVoucherStatus::Claimed),
        claimed_at: Set(now),
        expired_at: Set(now + Duration::days(1)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();

    let cfg = config();
    let state = AppState {
        db: db.clone(),
        redis: redis::Client::open(cfg.redis_url.clone()).unwrap(),
        storage: Storage::offline(&cfg),
        config: cfg.clone(),
    };
    let token = session::create_access(&cfg, &user_id.to_string(), &role_id.to_string()).unwrap();
    let app = crate::router(state.clone());
    let (status, body) = send(app.clone(), Method::GET, "/carts/user", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"][0]["qty"], 5);

    let payload = json!({"order":{"userId":user_id,"addressId":address_id,"voucherIds":[voucher_id,voucher_id]},
        "orderItems":[{"productId":product_id,"discountId":discount_id,"quantity":2}],
        "shipment":{"courier":"jne","service":"REG","cost":20.0,"estimated":"2026-11-11","description":null},
        "payment":{"method":"BCA"}});
    assert!(
        checkout::execute(&state, serde_json::from_value(payload.clone()).unwrap())
            .await
            .is_err(),
        "duplicate voucher should fail on order_vouchers PK after writes; entire transaction must rollback"
    );
    assert_eq!(
        inventory::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .reserved_quantity,
        0
    );
    assert_eq!(
        cart_item::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .quantity,
        5
    );
    assert_eq!(
        discount::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .redeemed_count,
        0
    );
    assert_eq!(order::Entity::find().all(&db).await.unwrap().len(), 0);
    assert_eq!(
        user_voucher::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .status,
        user_voucher::UserVoucherStatus::Claimed
    );

    let mut valid = payload;
    valid["order"]["voucherIds"] = json!([voucher_id]);
    let result = checkout::execute(&state, serde_json::from_value(valid).unwrap())
        .await
        .unwrap();
    assert!(
        result["message"]
            .as_str()
            .unwrap()
            .contains("Order created")
    );
    let order = order::Entity::find().one(&db).await.unwrap().unwrap();
    assert_eq!(
        (order.total_price, order.discount_amount, order.final_price),
        (200.0, 20.0, 195.0)
    );
    assert_eq!(order_item::Entity::find().all(&db).await.unwrap().len(), 1);
    assert_eq!(
        order_voucher::Entity::find().all(&db).await.unwrap().len(),
        1
    );
    let payment = payment::Entity::find().one(&db).await.unwrap().unwrap();
    assert_eq!(
        (payment.amount, payment.method, payment.method_type),
        (195.0, "BCA".into(), "BCA".into())
    );
    assert_eq!(
        shipment::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .cost,
        15.0
    );
    assert_eq!(
        inventory::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .reserved_quantity,
        2
    );
    assert_eq!(
        cart_item::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .quantity,
        3
    );
    assert_eq!(
        discount::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .redeemed_count,
        2
    );
    assert_eq!(
        user_voucher::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .status,
        user_voucher::UserVoucherStatus::Used
    );
    // Independent database connections really compete on PostgreSQL row locks.
    // With two units left, exactly one of two simultaneous checkouts can win.
    let mut concurrent_options = ConnectOptions::new(url);
    concurrent_options.max_connections(1).min_connections(1);
    let concurrent_db = Database::connect(concurrent_options).await.unwrap();
    concurrent_db
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("SET search_path TO \"{schema}\""),
        ))
        .await
        .unwrap();
    let concurrent_state = AppState {
        db: concurrent_db,
        ..state.clone()
    };
    let input: CheckoutInput = serde_json::from_value(json!({"order":{"userId":user_id,"addressId":address_id},
        "orderItems":[{"productId":product_id,"quantity":2}],
        "shipment":{"courier":"jne","service":"REG","cost":20,"estimated":"2026-11-11","description":null},
        "payment":{"method":"BCA"}}))
    .unwrap();
    let (first, second) = tokio::join!(
        checkout::execute(&state, input.clone()),
        checkout::execute(&concurrent_state, input)
    );
    assert!(
        first.is_ok() ^ second.is_ok(),
        "only one concurrent checkout can reserve remaining stock: {first:?} / {second:?}"
    );
    assert_eq!(
        inventory::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .reserved_quantity,
        4
    );
    assert_eq!(
        cart_item::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .quantity,
        1
    );
    assert_eq!(order::Entity::find().all(&db).await.unwrap().len(), 2);
    let (status, body) = send(app, Method::GET, "/payments", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        Value::Null,
        "Nest payment scaffold returns a plain text string"
    );

    root.execute_raw(Statement::from_string(
        DbBackend::Postgres,
        format!("DROP SCHEMA \"{schema}\" CASCADE"),
    ))
    .await
    .unwrap();
}
