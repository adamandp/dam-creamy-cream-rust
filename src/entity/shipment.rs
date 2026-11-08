use sea_orm::entity::prelude::*;

#[derive(
    Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, serde::Serialize, serde::Deserialize,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "SHIPMENT_STATUS")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ShipmentStatus {
    #[sea_orm(string_value = "PENDING")]
    Pending,
    #[sea_orm(string_value = "SHIPPED")]
    Shipped,
    #[sea_orm(string_value = "DELIVERED")]
    Delivered,
    #[sea_orm(string_value = "CANCELLED")]
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "shipments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub order_id: Uuid,
    pub courier: String,
    pub service: String,
    pub cost: f64,
    pub estimated: DateTime,
    pub delivered_at: Option<DateTime>,
    pub description: Option<String>,
    pub tracking_number: Option<String>,
    pub status: ShipmentStatus,
    pub updated_at: DateTime,
    pub created_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
