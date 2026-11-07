use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, serde::Serialize, serde::Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "VOUCHER_TYPE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VoucherType {
    #[sea_orm(string_value = "PERCENTAGE")]
    Percentage,
    #[sea_orm(string_value = "FIXED")]
    Fixed,
    #[sea_orm(string_value = "FREE_ITEM")]
    FreeItem,
}

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, serde::Serialize, serde::Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "VOUCHER_GROUP")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VoucherGroup {
    #[sea_orm(string_value = "CHECKOUT")]
    Checkout,
    #[sea_orm(string_value = "SHIPPING")]
    Shipping,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "vouchers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub r#type: VoucherType,
    pub group: VoucherGroup,
    pub value: Option<f64>,
    pub product_id: Option<Uuid>,
    pub quantity: Option<i32>,
    pub minimum_purchase: f64,
    pub is_active: bool,
    pub start_date: DateTime,
    pub end_date: DateTime,
    pub max_redeemed: i32,
    pub redeemed_count: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
