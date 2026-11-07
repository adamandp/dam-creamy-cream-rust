use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "addresses")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(column_name = "user_id")]
    pub user_id: Uuid,
    #[sea_orm(column_name = "recipient_name")]
    pub recipient_name: String,
    #[sea_orm(column_name = "recipient_phone_number")]
    pub recipient_phone_number: String,
    pub province: String,
    pub city: String,
    pub subdistrict: String,
    pub village: String,
    #[sea_orm(column_name = "postal_code")]
    pub postal_code: String,
    pub address: String,
    #[sea_orm(column_name = "is_primary")]
    pub is_primary: bool,
    #[sea_orm(column_name = "created_at")]
    pub created_at: DateTime,
    #[sea_orm(column_name = "updated_at")]
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
