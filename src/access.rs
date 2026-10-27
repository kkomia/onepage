use crate::db::Database;
use crate::error::{ApiError, ApiResult, AppError, AppResult};
use crate::models::{SpaceAccess, User};
use crate::services::spaces;

pub async fn require_page_space(
    db: &Database,
    user: &User,
    slug: &str,
) -> AppResult<SpaceAccess> {
    spaces::space_access(db, user, slug)
        .await?
        .ok_or_else(|| AppError::not_found("空间不存在，或你没有访问权限。"))
}

pub async fn require_page_admin(
    db: &Database,
    user: &User,
    slug: &str,
) -> AppResult<SpaceAccess> {
    let access = require_page_space(db, user, slug).await?;
    if access.is_admin(user) {
        Ok(access)
    } else {
        Err(AppError::forbidden("你需要空间管理员权限才能进入这里。"))
    }
}

pub fn require_super_admin_page(user: &User) -> AppResult<()> {
    if user.is_super_admin {
        Ok(())
    } else {
        Err(AppError::forbidden("这里只对系统管理员开放。"))
    }
}

pub async fn require_api_space(
    db: &Database,
    user: &User,
    slug: &str,
) -> ApiResult<SpaceAccess> {
    spaces::space_access(db, user, slug)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)
}

pub async fn require_api_member(
    db: &Database,
    user: &User,
    slug: &str,
) -> ApiResult<SpaceAccess> {
    let access = require_api_space(db, user, slug).await?;
    if access.is_member() {
        Ok(access)
    } else {
        Err(ApiError::forbidden())
    }
}

