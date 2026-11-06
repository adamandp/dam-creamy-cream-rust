use sea_orm::entity::prelude::*;

#[derive(
    Clone, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, serde::Serialize, serde::Deserialize,
)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "GENDER_TYPE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GenderType {
    #[sea_orm(string_value = "MALE")]
    Male,
    #[sea_orm(string_value = "FEMALE")]
    Female,
    #[sea_orm(string_value = "OTHER")]
    Other,
}

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(column_name = "role_id")]
    pub role_id: Uuid,
    pub username: String,
    pub email: Option<String>,
    #[sea_orm(column_name = "phone_number")]
    pub phone_number: Option<String>,
    pub password: String,
    #[sea_orm(column_name = "full_name")]
    pub full_name: String,
    #[sea_orm(column_name = "birth_date")]
    pub birth_date: Option<Date>,
    #[sea_orm(column_name = "image_url")]
    pub image_url: Option<String>,
    pub gender: Option<GenderType>,
    #[sea_orm(column_name = "is_active")]
    pub is_active: bool,
    #[sea_orm(column_name = "created_at")]
    pub created_at: DateTime,
    #[sea_orm(column_name = "updated_at")]
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
