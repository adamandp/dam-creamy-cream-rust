use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "user_tokens")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(column_name = "user_id")]
    pub user_id: Uuid,
    pub token: String,
    // These two columns are camelCase in the original Prisma migration.
    #[sea_orm(column_name = "browserInfo")]
    pub browser_info: String,
    #[sea_orm(column_name = "expiredAt")]
    pub expired_at: DateTime,
    #[sea_orm(column_name = "created_at")]
    pub created_at: DateTime,
    #[sea_orm(column_name = "updated_at")]
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
