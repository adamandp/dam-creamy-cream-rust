use sea_orm::entity::prelude::*;

#[derive(
    Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, serde::Serialize, serde::Deserialize,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "DISCOUNT_TYPE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DiscountType {
    #[sea_orm(string_value = "PERCENTAGE")]
    Percentage,
    #[sea_orm(string_value = "FIXED")]
    Fixed,
    #[sea_orm(string_value = "PRICE")]
    Price,
    #[sea_orm(string_value = "FREE_ITEM")]
    FreeItem,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "discounts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    pub discount_type: DiscountType,
    pub value: Option<f64>,
    pub max_applied: i32,
    pub max_redeemed: i32,
    pub redeemed_count: i32,
    pub description: Option<String>,
    pub product_id: Option<Uuid>,
    pub quantity: Option<i32>,
    pub start_date: DateTime,
    pub end_date: DateTime,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
