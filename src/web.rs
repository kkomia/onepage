use axum::http::HeaderMap;
use axum::response::Html;
use axum_extra::extract::cookie::CookieJar;
use serde_json::{json, Value};

use crate::auth;
use crate::error::{ApiError, ApiResult, AppError, AppResult};
use crate::models::{SessionUser, Space, User};
use crate::services::settings;
use crate::state::AppState;
use crate::templates;

pub async fn site_name(state: &AppState) -> AppResult<String> {
    settings::site_name(&state.db).await.map_err(Into::into)
}

pub fn login_redirect(next: &str) -> AppError {
    let encoded: String = url::form_urlencoded::byte_serialize(next.as_bytes()).collect();
    AppError::Redirect(format!("/login?next={encoded}"))
}

pub async fn session_optional(state: &AppState, jar: &CookieJar) -> Option<SessionUser> {
    auth::session_from_jar(&state.db, jar).await
}

pub async fn require_page_session(
    state: &AppState,
    jar: &CookieJar,
    next: &str,
) -> AppResult<SessionUser> {
    session_optional(state, jar)
        .await
        .ok_or_else(|| login_redirect(next))
}

pub async fn require_ready_page_session(
    state: &AppState,
    jar: &CookieJar,
    next: &str,
) -> AppResult<SessionUser> {
    let session = require_page_session(state, jar, next).await?;
    if session.user.must_change_password {
        return Err(AppError::Redirect("/account/password?required=1".to_string()));
    }
    Ok(session)
}

pub async fn require_api_session(state: &AppState, jar: &CookieJar) -> ApiResult<SessionUser> {
    let session = session_optional(state, jar)
        .await
        .ok_or_else(ApiError::unauthorized)?;
    if session.user.must_change_password {
        return Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "请先修改初始密码",
        ));
    }
    Ok(session)
}

pub fn verify_page_csrf(session: &SessionUser, token: &str) -> AppResult<()> {
    if auth::verify_csrf(session, token) {
        Ok(())
    } else {
        Err(AppError::bad_request("页面已过期，请刷新后重试。"))
    }
}

pub fn verify_api_csrf(session: &SessionUser, headers: &HeaderMap) -> ApiResult<()> {
    if auth::verify_api_csrf(session, headers) {
        Ok(())
    } else {
        Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "页面已过期，请刷新后重试",
        ))
    }
}

pub fn render_page(state: &AppState, name: &str, context: Value) -> AppResult<Html<String>> {
    templates::render(&state.templates, name, context)
}

pub fn base_context(site_name: &str, page: &str) -> Value {
    templates::base_context(site_name, page)
}

pub fn auth_context(site_name: &str, page: &str) -> Value {
    base_context(site_name, page)
}

pub fn user_context(user: &User) -> Value {
    json!({
        "id": user.id,
        "username": user.username,
        "display_name": user.display_name,
        "is_super_admin": user.is_super_admin,
        "is_active": user.is_active,
        "must_change_password": user.must_change_password,
        "last_login_at": user.last_login_at,
        "initials": user.initials(),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn admin_context(
    state_site_name: &str,
    page_kicker: &str,
    page_title: &str,
    page_desc: &str,
    nav: &str,
    user: &User,
    csrf: &str,
    is_global: bool,
    space: Option<&Space>,
    flash: Option<Value>,
) -> Value {
    let mut context = base_context(state_site_name, "admin");
    context["page_title"] = json!(page_title);
    context["page_kicker"] = json!(page_kicker);
    context["page_desc"] = json!(page_desc);
    context["nav"] = json!(nav);
    context["user"] = user_context(user);
    context["user_initials"] = json!(user.initials());
    context["csrf"] = json!(csrf);
    context["is_global"] = json!(is_global);
    context["space"] = match space {
        Some(space) => json!({
            "id": space.id,
            "slug": space.slug,
            "name": space.name,
            "description": space.description,
            "created_at": space.created_at,
        }),
        None => Value::Null,
    };
    context["flash"] = flash.unwrap_or(Value::Null);
    context
}

pub fn flash_from_query(ok: Option<&str>, err: Option<&str>) -> Option<Value> {
    if let Some(code) = ok {
        if let Some(message) = success_message(code) {
            return Some(json!({ "kind": "success", "message": message }));
        }
    }
    if let Some(code) = err {
        if let Some(message) = error_message(code) {
            return Some(json!({ "kind": "error", "message": message }));
        }
    }
    None
}

fn success_message(code: &str) -> Option<&'static str> {
    Some(match code {
        "link_created" => "链接已添加。",
        "link_updated" => "链接已更新。",
        "link_deleted" => "链接已删除。",
        "link_moved" => "链接顺序已更新。",
        "icon_refreshed" => "图标已刷新。",
        "category_created" => "分类已添加。",
        "category_updated" => "分类名称已更新。",
        "category_deleted" => "分类已删除，原有链接已移到未分类。",
        "category_moved" => "分类顺序已更新。",
        "member_added" => "成员已加入空间。",
        "member_removed" => "成员已移出空间。",
        "role_updated" => "成员角色已更新。",
        "space_updated" => "空间信息已保存。",
        "space_created" => "空间已创建。",
        "user_created" => "账号已创建。",
        "user_enabled" => "账号已启用。",
        "user_disabled" => "账号已停用。",
        "password_reset" => "密码已重置，对方下次登录需要修改。",
        "site_updated" => "站点名称已保存。",
        _ => return None,
    })
}

fn error_message(code: &str) -> Option<&'static str> {
    Some(match code {
        "last_admin" => "空间至少需要保留一位管理员。",
        "member_missing" => "这个账号不存在，或已经停用。",
        "member_exists" => "这个账号已经是空间成员。",
        "category_duplicate" => "这个分类已经存在。",
        "category_missing" => "分类不存在。",
        "link_missing" => "链接不存在。",
        "save_failed" => "保存时遇到问题，请检查内容后重试。",
        "user_missing" => "账号不存在。",
        "user_self" => "不能在这里停用自己的账号。",
        "last_super_admin" => "必须保留至少一位可用的系统管理员。",
        "self_remove" => "不能在这个页面把自己移出空间。",
        "self_role" => "不能在这个页面降低自己的角色。",
        "icon_failed" => "没有抓到可用图标，稍后可以再试一次。",
        "session_expired" => "页面已过期，请重新操作。",
        _ => return None,
    })
}

