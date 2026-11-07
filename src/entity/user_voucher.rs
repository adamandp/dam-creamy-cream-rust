use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, serde::Serialize, serde::Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "USER_VOUCHER_STATUS")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UserVoucherStatus {
    #[sea_orm(string_value = "USED")]
    Used,
    #[sea_orm(string_value = "CLAIMED")]
    Claimed,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "user_vouchers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub voucher_id: Uuid,
    pub status: UserVoucherStatus,
    pub claimed_at: DateTime,
    pub expired_at: DateTime,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
