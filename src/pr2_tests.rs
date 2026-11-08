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
    entity::{
        inventory, order_item, product, product_information, review, role, user, user_voucher,
    },
    features::session,
    infrastructure::storage::Storage,
    state::AppState,
};

#[test]
fn dto_shape_matches_nest_zod_strict_nullable_and_coercion() {
    use crate::features::{
        addresses, categories, discounts, product_discounts, products, reviews, user_vouchers,
        users, vouchers,
    };

    let category = json!({"name":"Cream Cake","description":null,"isActive":true,"ignoredByZod":4});
    assert!(serde_json::from_value::<categories::Create>(category.clone()).is_ok());
    let mut missing_category_description = category;
    missing_category_description
        .as_object_mut()
        .unwrap()
        .remove("description");
    assert!(serde_json::from_value::<categories::Create>(missing_category_description).is_err());
    assert!(
        serde_json::from_value::<categories::Update>(json!({"unused":"ignored","name":"Cream"}))
            .is_ok()
    );

    let product = json!({"categoryId":Uuid::new_v4(),"discountId":null,"name":"Cream","description":null,"price":1100.0,"isActive":true});
    assert!(serde_json::from_value::<products::Create>(product.clone()).is_ok());
    let mut missing_product_description = product.clone();
    missing_product_description
        .as_object_mut()
        .unwrap()
        .remove("description");
    assert!(serde_json::from_value::<products::Create>(missing_product_description).is_err());
    let mut extra_product_field = product;
    extra_product_field
        .as_object_mut()
        .unwrap()
        .insert("extra".into(), json!(true));
    assert!(serde_json::from_value::<products::Create>(extra_product_field).is_err());

    let discount = json!({"name":"Cream Sale","discountType":"FIXED","value":null,"maxApplied":1,"maxRedeemed":10,"redeemedCount":0,"description":null,"productId":null,"quantity":null,"startDate":"2026-11-08T00:00:00Z","endDate":"2026-12-08T00:00:00Z","ignoredByZod":"okay"});
    assert!(serde_json::from_value::<discounts::Create>(discount.clone()).is_ok());
    for required in ["value", "description", "productId", "quantity"] {
        let mut missing = discount.clone();
        missing.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<discounts::Create>(missing).is_err(),
            "missing {required} must be rejected"
        );
    }

    let voucher = json!({"code":"CREAM10","name":"Cream Sale","description":null,"type":"FIXED","group":"CHECKOUT","value":null,"productId":null,"quantity":null,"minimumPurchase":1000.0,"isActive":true,"startDate":"2026-11-08T00:00:00Z","endDate":"2026-12-08T00:00:00Z","maxRedeemed":10,"redeemedCount":0,"unknown":true});
    assert!(serde_json::from_value::<vouchers::Create>(voucher.clone()).is_ok());
    for required in ["description", "value", "productId", "quantity"] {
        let mut missing = voucher.clone();
        missing.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<vouchers::Create>(missing).is_err(),
            "missing {required} must be rejected"
        );
    }

    assert!(
        serde_json::from_value::<reviews::Create>(
            json!({"userId":Uuid::new_v4(),"productId":Uuid::new_v4(),"rating":4,"unknown":true})
        )
        .is_ok()
    );
    assert!(
        serde_json::from_value::<product_discounts::Create>(
            json!({"productId":Uuid::new_v4(),"discountId":Uuid::new_v4(),"unknown":true})
        )
        .is_ok()
    );
    assert!(serde_json::from_value::<user_vouchers::Create>(json!({"userId":Uuid::new_v4(),"voucherId":Uuid::new_v4(),"claimedAt":"2026-11-08T00:00:00Z","expiredAt":"2026-12-08T00:00:00Z","unknown":true})).is_ok());

    let address = json!({"userId":Uuid::new_v4(),"recipientName":"Adam","recipientPhoneNumber":"081234567890","province":"Jawa Barat","city":"Bandung","subdistrict":"Andir","village":"Ciroyom","postalCode":"40182","address":"Jl Cream","isPrimary":"false"});
    let parsed = serde_json::from_value::<addresses::CreateAddressBody>(address.clone()).unwrap();
    assert!(parsed.is_primary, "Zod coerce.boolean('false') is true");
    let parsed =
        serde_json::from_value::<addresses::UpdateAddressBody>(json!({"isPrimary":null})).unwrap();
    assert_eq!(parsed.is_primary, Some(false));
    let mut extra_address = address;
    extra_address
        .as_object_mut()
        .unwrap()
        .insert("extra".into(), json!(true));
    assert!(serde_json::from_value::<addresses::CreateAddressBody>(extra_address).is_err());

    let user = json!({"username":"creamuser","fullName":"Cream User","password":"Abcdef1!","roleId":Uuid::new_v4(),"unexpected":true});
    assert!(serde_json::from_value::<users::CreateUserBody>(user).is_err());
}

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
        "CREATE TABLE order_items (id UUID PRIMARY KEY,order_id UUID NOT NULL DEFAULT gen_random_uuid(),product_id UUID NOT NULL REFERENCES products(id),discount_id UUID,discount_amount DOUBLE PRECISION NOT NULL DEFAULT 0,price DOUBLE PRECISION NOT NULL DEFAULT 0,final_price DOUBLE PRECISION NOT NULL DEFAULT 0,quantity INTEGER NOT NULL,created_at TIMESTAMP NOT NULL DEFAULT now(),updated_at TIMESTAMP NOT NULL DEFAULT now())",
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
        authorized,
        Some(json!({"name":"Ice-Cream","description":"Creamy","isActive":true})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST); // Nest baseName disallows punctuation.
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
    // Nest ProductDiscountsService.findAll counts *users* rather than links.
    // Introduce a second user so these totals cannot accidentally match.
    let second_user_id = Uuid::new_v4();
    user::ActiveModel {
        id: Set(second_user_id),
        role_id: Set(role_id),
        username: Set("secondtester".into()),
        email: Set(None),
        phone_number: Set(None),
        password: Set("password-placeholder".into()),
        full_name: Set("Second Tester".into()),
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
    let (status, links) = send(app.clone(), Method::GET, "/product-discounts", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(links["data"].as_array().unwrap().len(), 1);
    assert_eq!(links["paging"]["totalItems"], 2);
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
    let (status, paged_catalog) = send(
        app.clone(),
        Method::GET,
        "/products/catalog?page=1&limit=10",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(paged_catalog["paging"]["currentPage"], 1);
    assert_eq!(paged_catalog["paging"]["pageSize"], 10);
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
    // A review on another product must affect the legacy *global* pagination
    // count without appearing in the current product's filtered rows.
    let other_product_id = Uuid::new_v4();
    product::ActiveModel {
        id: Set(other_product_id),
        category_id: Set(cat.id),
        discount_id: Set(None),
        name: Set("Other Flavor".into()),
        image_url: Set(None),
        description: Set(None),
        price: Set(1000.0),
        is_active: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
    review::ActiveModel {
        id: Set(Uuid::new_v4()),
        user_id: Set(second_user_id),
        product_id: Set(other_product_id),
        rating: Set(4),
        comment: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&db)
    .await
    .unwrap();
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
    assert_eq!(reviews["data"].as_array().unwrap().len(), 1);
    assert_eq!(reviews["paging"]["totalItems"], 2);
    let (status, mine) = send(app.clone(), Method::GET, "/reviews/me", authorized, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mine["data"].as_array().unwrap().len(), 1);
    assert_eq!(mine["paging"]["totalItems"], 2);
    let inv = inventory::Entity::find().one(&db).await.unwrap().unwrap();
    let (status, _) = send(
        app.clone(),
        Method::PATCH,
        &format!("/inventory/{}", inv.id),
        authorized,
        Some(json!({"reservedQuantity":21})),
    )
    .await;
    assert_eq!(status, StatusCode::OK); // Legacy inventory PATCH has no reserved <= quantity guard.
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
        app.clone(),
        Method::GET,
        &format!("/user-vouchers/{}", uv.id),
        authorized,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["status"], "USED");
    let (status, _) = send(
        app.clone(),
        Method::POST,
        "/user-vouchers",
        authorized,
        Some(json!({"userId":user_id,"voucherId":voucher.id,"claimedAt":"2026-12-01T00:00:00Z","expiredAt":"2026-11-01T00:00:00Z"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    // The Nest DTOs allow negative promotion numbers and reversed date order.
    let (status, _) = send(
        app,
        Method::POST,
        "/discounts",
        authorized,
        Some(json!({"name":"Legacy Numbers","discountType":"FIXED","value":-10.0,"maxApplied":-1,"maxRedeemed":-1,"redeemedCount":2,"description":null,"productId":null,"quantity":-5,"startDate":"2027-01-01T00:00:00Z","endDate":"2026-01-01T00:00:00Z"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    drop(db);
    admin_db
        .execute_raw(Statement::from_string(
            DbBackend::Postgres,
            format!("DROP SCHEMA \"{schema}\" CASCADE"),
        ))
        .await
        .unwrap();
}
