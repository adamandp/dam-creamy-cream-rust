use axum::http::HeaderMap;
use sea_orm::EntityTrait;
use uuid::Uuid;

use super::session;
use crate::{
    entity::{role, user},
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Clone, Debug)]
pub struct Actor {
    pub id: Uuid,
    pub is_admin: bool,
    pub is_super_admin: bool,
}

impl Actor {
    pub fn owns_or_administers(&self, user_id: Uuid) -> bool {
        self.id == user_id || self.is_admin
    }

    pub fn require_owner_or_admin(&self, user_id: Uuid) -> AppResult<()> {
        if self.owns_or_administers(user_id) {
            Ok(())
        } else {
            Err(AppError::Forbidden("Cannot access another user's resources".into()))
        }
    }

    pub fn require_admin(&self) -> AppResult<()> {
        if self.is_admin {
            Ok(())
        } else {
            Err(AppError::Forbidden("Administrator access required".into()))
        }
    }

    pub fn require_super_admin(&self) -> AppResult<()> {
        if self.is_super_admin {
            Ok(())
        } else {
            Err(AppError::Forbidden("Super administrator access required".into()))
        }
    }
}

pub async fn authenticate(state: &AppState, headers: &HeaderMap) -> AppResult<Actor> {
    let claims = session::bearer(headers, &state.config)?;
    let id = Uuid::parse_str(&claims.sub)
        .map_err(|_| AppError::Unauthorized("Invalid access token subject".into()))?;
    let current_user = user::Entity::find_by_id(id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::Unauthorized("Account no longer exists".into()))?;
    // Never authorize using a potentially stale role claim from a previously issued JWT.
    let current_role = role::Entity::find_by_id(current_user.role_id)
        .one(&state.db)
        .await
        .map_err(AppError::db)?
        .ok_or_else(|| AppError::Unauthorized("Account role no longer exists".into()))?;
    let is_super_admin = current_role.name == "super-admin";
    Ok(Actor {
        id,
        is_admin: is_super_admin || current_role.name == "admin",
        is_super_admin,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customers_can_only_mutate_their_own_resources() {
        let actor = Actor {
            id: Uuid::new_v4(),
            is_admin: false,
            is_super_admin: false,
        };
        assert!(actor.require_owner_or_admin(actor.id).is_ok());
        assert!(matches!(actor.require_owner_or_admin(Uuid::new_v4()), Err(AppError::Forbidden(_))));
        assert!(matches!(actor.require_admin(), Err(AppError::Forbidden(_))));
        assert!(matches!(actor.require_super_admin(), Err(AppError::Forbidden(_))));
    }

    #[test]
    fn only_super_admins_can_manage_roles() {
        let admin = Actor {
            id: Uuid::new_v4(),
            is_admin: true,
            is_super_admin: false,
        };
        assert!(admin.require_admin().is_ok());
        assert!(admin.require_super_admin().is_err());
        assert!(admin.require_owner_or_admin(Uuid::new_v4()).is_ok());
        let root = Actor { is_super_admin: true, ..admin };
        assert!(root.require_super_admin().is_ok());
    }
}
