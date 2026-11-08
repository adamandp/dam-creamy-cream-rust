use sea_orm::entity::prelude::*;

#[derive(
    Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, serde::Serialize, serde::Deserialize,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "PAYMENT_STATUS")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentStatus {
    #[sea_orm(string_value = "PENDING")]
    Pending,
    #[sea_orm(string_value = "PAID")]
    Paid,
    #[sea_orm(string_value = "FAILED")]
    Failed,
    #[sea_orm(string_value = "CANCELLED")]
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "payments")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub order_id: Uuid,
    pub method: String,
    pub method_type: String,
    pub amount: f64,
    pub status: PaymentStatus,
    pub transaction_code: Option<String>,
    pub transaction_id: Option<String>,
    pub expiry_date: Option<DateTime>,
    pub paid_at: Option<DateTime>,
    pub updated_at: DateTime,
    pub created_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
