use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "order_vouchers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub order_id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub voucher_id: Uuid,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
