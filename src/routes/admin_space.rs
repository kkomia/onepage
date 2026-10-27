use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::access;
use crate::error::{AppError, AppResult};
use crate::models::{LinkRecord, Role, SessionUser, SpaceAccess};
use crate::services::{favicon, links, spaces};
use crate::state::AppState;
use crate::util::{
    format_date, host_label, normalize_url, pinyin_index, validate_category_name,
    validate_description, validate_link_title, validate_space_name,
};
use crate::web;

#[derive(Debug, Deserialize, Default)]
pub struct LinksQuery {
    q: Option<String>,
    dialog: Option<String>,
    edit: Option<i64>,
    ok: Option<String>,
    err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LinkForm {
    csrf: String,
    title: String,
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    category_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

#[derive(Debug, Deserialize)]
pub struct MoveForm {
    csrf: String,
    dir: String,
}

#[derive(Debug, Deserialize)]
pub struct CategoryForm {
    csrf: String,
    name: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct CategoriesQuery {
    ok: Option<String>,
    err: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct MembersQuery {
    ok: Option<String>,
    err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddMemberForm {
    csrf: String,
    username: String,
    role: String,
}

#[derive(Debug, Deserialize)]
pub struct RoleForm {
    csrf: String,
    role: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SettingsQuery {
    ok: Option<String>,
    err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    csrf: String,
    name: String,
    #[serde(default)]
    description: String,
}

pub async fn index(Path(space_slug): Path<String>) -> Redirect {
    Redirect::to(&format!("/s/{space_slug}/admin/links"))
}

pub async fn links_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Query(query): Query<LinksQuery>,
) -> Response {
    match links_page_inner(state, jar, space_slug, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn links_page_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    query: LinksQuery,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/links");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    let flash = web::flash_from_query(query.ok.as_deref(), query.err.as_deref());

    let mut dialog = None;
    if query.dialog.as_deref() == Some("new") {
        dialog = Some(dialog_new(&access));
    } else if query.dialog.as_deref() == Some("edit") {
        if let Some(link_id) = query.edit {
            if let Some(link) = links::link_by_id(&state.db, link_id).await? {
                if link.space_id == access.space.id && link.owner_id.is_none() {
                    dialog = Some(dialog_edit(&access, &link, None));
                }
            }
        }
    }

    render_links(&state, &session, &access, &query, dialog, flash, StatusCode::OK).await
}

pub async fn create_link(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Form(form): Form<LinkForm>,
) -> Response {
    match create_link_inner(state, jar, space_slug, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn create_link_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    form: LinkForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/links");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;

    let parsed = parse_link_form(&form, &state, &access).await;
    let (title, url, description, category_id) = match parsed {
        Ok(value) => value,
        Err(error) => {
            let query = LinksQuery::default();
            let dialog = dialog_from_form(&access, &form, Some(&error), None);
            return render_links(
                &state,
                &session,
                &access,
                &query,
                Some(dialog),
                None,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        }
    };

    let created = links::create_link(
        &state.db,
        links::NewLink {
            space_id: access.space.id,
            owner_id: None,
            category_id,
            title: &title,
            url: &url,
            description: &description,
            created_by: session.user.id,
        },
    )
    .await;
    let created = match created {
        Ok(link) => link,
        Err(error) => {
            tracing::error!(error = %error, "failed to create public link");
            let query = LinksQuery::default();
            let dialog = dialog_from_form(&access, &form, Some("保存时遇到问题，请稍后重试。"), None);
            return render_links(
                &state,
                &session,
                &access,
                &query,
                Some(dialog),
                None,
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .await;
        }
    };
    state.refresh_icon_in_background(created.id);
    Ok(Redirect::to(&format!("{next}?ok=link_created")).into_response())
}

pub async fn update_link(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, link_id)): Path<(String, i64)>,
    Form(form): Form<LinkForm>,
) -> Response {
    match update_link_inner(state, jar, space_slug, link_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn update_link_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    link_id: i64,
    form: LinkForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/links");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let Some(existing) = links::link_by_id(&state.db, link_id).await? else {
        return Err(AppError::not_found("链接不存在。"));
    };
    if existing.space_id != access.space.id || existing.owner_id.is_some() {
        return Err(AppError::not_found("链接不存在。"));
    }

    let parsed = parse_link_form(&form, &state, &access).await;
    let (title, url, description, category_id) = match parsed {
        Ok(value) => value,
        Err(error) => {
            let query = LinksQuery::default();
            let dialog = dialog_from_form(&access, &form, Some(&error), Some(existing.id));
            return render_links(
                &state,
                &session,
                &access,
                &query,
                Some(dialog),
                None,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        }
    };

    let url_changed = existing.url != url;
    let updated = links::update_link(
        &state.db,
        &existing,
        links::LinkUpdate {
            category_id,
            title: &title,
            url: &url,
            description: &description,
        },
    )
    .await?;
    if url_changed || updated.icon_hash.is_none() {
        state.refresh_icon_in_background(updated.id);
    }
    Ok(Redirect::to(&format!("{next}?ok=link_updated")).into_response())
}

pub async fn delete_link(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, link_id)): Path<(String, i64)>,
    Form(form): Form<CsrfForm>,
) -> Response {
    match delete_link_inner(state, jar, space_slug, link_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn delete_link_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    link_id: i64,
    form: CsrfForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/links");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let Some(link) = links::link_by_id(&state.db, link_id).await? else {
        return Err(AppError::not_found("链接不存在。"));
    };
    if link.space_id != access.space.id || link.owner_id.is_some() {
        return Err(AppError::not_found("链接不存在。"));
    }
    links::delete_link(&state.db, link_id).await?;
    Ok(Redirect::to(&format!("{next}?ok=link_deleted")).into_response())
}

pub async fn move_link(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, link_id)): Path<(String, i64)>,
    Form(form): Form<MoveForm>,
) -> Response {
    match move_link_inner(state, jar, space_slug, link_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn move_link_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    link_id: i64,
    form: MoveForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/links");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    if form.dir != "up" && form.dir != "down" {
        return Err(AppError::bad_request("未知的排序方向。"));
    }
    let Some(link) = links::link_by_id(&state.db, link_id).await? else {
        return Err(AppError::not_found("链接不存在。"));
    };
    if link.space_id != access.space.id || link.owner_id.is_some() {
        return Err(AppError::not_found("链接不存在。"));
    }
    links::move_link(&state.db, link_id, &form.dir).await?;
    Ok(Redirect::to(&format!("{next}?ok=link_moved")).into_response())
}

pub async fn refresh_link_icon(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, link_id)): Path<(String, i64)>,
    Form(form): Form<CsrfForm>,
) -> Response {
    match refresh_link_icon_inner(state, jar, space_slug, link_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn refresh_link_icon_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    link_id: i64,
    form: CsrfForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/links");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let Some(link) = links::link_by_id(&state.db, link_id).await? else {
        return Err(AppError::not_found("链接不存在。"));
    };
    if link.space_id != access.space.id || link.owner_id.is_some() {
        return Err(AppError::not_found("链接不存在。"));
    }
    let hash = refresh_icon_sync(&state, link_id).await;
    let target = if hash.is_some() {
        format!("{next}?ok=icon_refreshed")
    } else {
        format!("{next}?err=icon_failed")
    };
    Ok(Redirect::to(&target).into_response())
}

pub async fn categories_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Query(query): Query<CategoriesQuery>,
) -> Response {
    match categories_page_inner(state, jar, space_slug, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn categories_page_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    query: CategoriesQuery,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/categories");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    let flash = web::flash_from_query(query.ok.as_deref(), query.err.as_deref());
    render_categories(&state, &session, &access, flash, None, None, StatusCode::OK).await
}

pub async fn create_category(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Form(form): Form<CategoryForm>,
) -> Response {
    match create_category_inner(state, jar, space_slug, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn create_category_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    form: CategoryForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/categories");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let name = match validate_category_name(&form.name) {
        Ok(name) => name,
        Err(error) => {
            return render_categories(
                &state,
                &session,
                &access,
                None,
                Some(&error),
                Some(&form.name),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
    };
    match links::create_category(&state.db, access.space.id, None, &name).await {
        Ok(_) => Ok(Redirect::to(&format!("{next}?ok=category_created")).into_response()),
        Err(error) => {
            let duplicate = error.to_string().contains("已经存在");
            let code = if duplicate { "category_duplicate" } else { "save_failed" };
            Ok(Redirect::to(&format!("{next}?err={code}")).into_response())
        }
    }
}

pub async fn rename_category(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, category_id)): Path<(String, i64)>,
    Form(form): Form<CategoryForm>,
) -> Response {
    match rename_category_inner(state, jar, space_slug, category_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn rename_category_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    category_id: i64,
    form: CategoryForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/categories");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let Some(category) = links::category_by_id(&state.db, category_id).await? else {
        return Ok(Redirect::to(&format!("{next}?err=category_missing")).into_response());
    };
    if category.space_id != access.space.id || category.owner_id.is_some() {
        return Ok(Redirect::to(&format!("{next}?err=category_missing")).into_response());
    }
    let name = match validate_category_name(&form.name) {
        Ok(name) => name,
        Err(_) => return Ok(Redirect::to(&format!("{next}?err=save_failed")).into_response()),
    };
    match links::rename_category(&state.db, category_id, &name).await {
        Ok(_) => Ok(Redirect::to(&format!("{next}?ok=category_updated")).into_response()),
        Err(error) => {
            let code = if error.to_string().contains("已经存在") {
                "category_duplicate"
            } else {
                "save_failed"
            };
            Ok(Redirect::to(&format!("{next}?err={code}")).into_response())
        }
    }
}

pub async fn delete_category(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, category_id)): Path<(String, i64)>,
    Form(form): Form<CsrfForm>,
) -> Response {
    match delete_category_inner(state, jar, space_slug, category_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn delete_category_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    category_id: i64,
    form: CsrfForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/categories");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let Some(category) = links::category_by_id(&state.db, category_id).await? else {
        return Ok(Redirect::to(&format!("{next}?err=category_missing")).into_response());
    };
    if category.space_id != access.space.id || category.owner_id.is_some() {
        return Ok(Redirect::to(&format!("{next}?err=category_missing")).into_response());
    }
    links::delete_category(&state.db, category_id).await?;
    Ok(Redirect::to(&format!("{next}?ok=category_deleted")).into_response())
}

pub async fn move_category(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, category_id)): Path<(String, i64)>,
    Form(form): Form<MoveForm>,
) -> Response {
    match move_category_inner(state, jar, space_slug, category_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn move_category_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    category_id: i64,
    form: MoveForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/categories");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    if form.dir != "up" && form.dir != "down" {
        return Err(AppError::bad_request("未知的排序方向。"));
    }
    let Some(category) = links::category_by_id(&state.db, category_id).await? else {
        return Ok(Redirect::to(&format!("{next}?err=category_missing")).into_response());
    };
    if category.space_id != access.space.id || category.owner_id.is_some() {
        return Ok(Redirect::to(&format!("{next}?err=category_missing")).into_response());
    }
    links::move_category(&state.db, category_id, &form.dir).await?;
    Ok(Redirect::to(&format!("{next}?ok=category_moved")).into_response())
}

pub async fn members_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Query(query): Query<MembersQuery>,
) -> Response {
    match members_page_inner(state, jar, space_slug, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn members_page_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    query: MembersQuery,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/members");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    let flash = web::flash_from_query(query.ok.as_deref(), query.err.as_deref());
    render_members(&state, &session, &access, flash, None, None, None, StatusCode::OK).await
}

pub async fn add_member(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Form(form): Form<AddMemberForm>,
) -> Response {
    match add_member_inner(state, jar, space_slug, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn add_member_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    form: AddMemberForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/members");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;

    let role = if form.role == "admin" {
        Role::Admin
    } else {
        Role::Member
    };
    let username = form.username.trim();
    if username.is_empty() {
        return render_members(
            &state,
            &session,
            &access,
            None,
            Some("请填写成员账号。"),
            Some(username),
            Some(form.role.as_str()),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }

    match spaces::add_member(&state.db, access.space.id, username, role).await {
        Ok(_) => Ok(Redirect::to(&format!("{next}?ok=member_added")).into_response()),
        Err(error) => {
            let message = error.to_string();
            let code = if message.contains("找不到") || message.contains("停用") {
                "member_missing"
            } else if message.contains("已经是") {
                "member_exists"
            } else {
                "save_failed"
            };
            Ok(Redirect::to(&format!("{next}?err={code}")).into_response())
        }
    }
}

pub async fn set_role(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, user_id)): Path<(String, i64)>,
    Form(form): Form<RoleForm>,
) -> Response {
    match set_role_inner(state, jar, space_slug, user_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn set_role_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    user_id: i64,
    form: RoleForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/members");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    if user_id == session.user.id && form.role != "admin" {
        return Ok(Redirect::to(&format!("{next}?err=self_role")).into_response());
    }
    let role = if form.role == "admin" {
        Role::Admin
    } else {
        Role::Member
    };
    match spaces::set_member_role(&state.db, access.space.id, user_id, role).await {
        Ok(_) => Ok(Redirect::to(&format!("{next}?ok=role_updated")).into_response()),
        Err(error) => {
            let code = if error.to_string().contains("至少需要") {
                "last_admin"
            } else {
                "save_failed"
            };
            Ok(Redirect::to(&format!("{next}?err={code}")).into_response())
        }
    }
}

pub async fn remove_member(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, user_id)): Path<(String, i64)>,
    Form(form): Form<CsrfForm>,
) -> Response {
    match remove_member_inner(state, jar, space_slug, user_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn remove_member_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    user_id: i64,
    form: CsrfForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/members");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    if user_id == session.user.id {
        return Ok(Redirect::to(&format!("{next}?err=self_remove")).into_response());
    }
    match spaces::remove_member(&state.db, access.space.id, user_id).await {
        Ok(_) => Ok(Redirect::to(&format!("{next}?ok=member_removed")).into_response()),
        Err(error) => {
            let code = if error.to_string().contains("至少需要") {
                "last_admin"
            } else {
                "save_failed"
            };
            Ok(Redirect::to(&format!("{next}?err={code}")).into_response())
        }
    }
}

pub async fn settings_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Query(query): Query<SettingsQuery>,
) -> Response {
    match settings_page_inner(state, jar, space_slug, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn settings_page_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    query: SettingsQuery,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/settings");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    let flash = web::flash_from_query(query.ok.as_deref(), query.err.as_deref());
    render_settings(&state, &session, &access, flash, StatusCode::OK).await
}

pub async fn update_settings(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Form(form): Form<SettingsForm>,
) -> Response {
    match update_settings_inner(state, jar, space_slug, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn update_settings_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    form: SettingsForm,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}/admin/settings");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_admin(&state.db, &session.user, &space_slug).await?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let name = match validate_space_name(&form.name) {
        Ok(name) => name,
        Err(_) => return Ok(Redirect::to(&format!("{next}?err=save_failed")).into_response()),
    };
    let description = match validate_description(&form.description) {
        Ok(description) => description,
        Err(_) => return Ok(Redirect::to(&format!("{next}?err=save_failed")).into_response()),
    };
    spaces::update_space(&state.db, access.space.id, &name, &description).await?;
    Ok(Redirect::to(&format!("{next}?ok=space_updated")).into_response())
}

async fn render_links(
    state: &AppState,
    session: &SessionUser,
    access: &SpaceAccess,
    query: &LinksQuery,
    dialog: Option<Value>,
    flash: Option<Value>,
    status: StatusCode,
) -> AppResult<Response> {
    let site_name = web::site_name(state).await?;
    let public_links = links::links_for(&state.db, access.space.id, None).await?;
    let public_categories = links::categories_for(&state.db, access.space.id, None).await?;
    let q = query.q.as_deref().unwrap_or("").trim().to_string();
    let category_names: HashMap<i64, String> = public_categories
        .iter()
        .map(|category| (category.id, category.name.clone()))
        .collect();

    let total = public_links.len() as i64;
    let needle = q.to_lowercase();
    let mut rows = Vec::new();
    for link in &public_links {
        let category_name = link
            .category_id
            .and_then(|id| category_names.get(&id).cloned())
            .unwrap_or_else(|| "未分类".to_string());
        if !needle.is_empty() {
            // Same index the member view searches, so "sjzl" works here too.
            let (py, pyi) = pinyin_index(&format!("{} {}", link.title, category_name));
            let haystack = format!(
                "{} {} {} {} {py} {pyi}",
                link.title,
                link.url,
                link.description,
                category_name
            )
            .to_lowercase();
            if !haystack.contains(&needle) {
                continue;
            }
        }
        rows.push(json!({
            "id": link.id,
            "title": link.title,
            "url": link.url,
            "host": host_label(&link.url),
            "icon_url": link.icon_hash.as_ref().map(|hash| format!("/icons/{hash}")),
            "tone": link.id.rem_euclid(6),
            "initial": initial_of(&link.title),
            "category_name": category_name,
            "click_count": link.click_count,
            "updated_at_label": format_date(&link.updated_at),
        }));
    }

    let mut context = web::admin_context(
        &site_name,
        "空间管理",
        "公共导航",
        "这些入口会出现在空间成员的公共导航里。",
        "links",
        &session.user,
        &session.csrf,
        false,
        Some(&access.space),
        flash,
    );
    context["links"] = json!(rows);
    context["categories"] = json!(public_categories
        .iter()
        .map(|category| json!({ "id": category.id, "name": category.name }))
        .collect::<Vec<_>>());
    context["query"] = json!(q);
    context["total"] = json!(total);
    context["filtered"] = json!(rows.len());
    context["dialog"] = dialog.unwrap_or(Value::Null);
    let html = web::render_page(state, "admin/links.html", context)?;
    Ok((status, html).into_response())
}

async fn render_categories(
    state: &AppState,
    session: &SessionUser,
    access: &SpaceAccess,
    flash: Option<Value>,
    add_error: Option<&str>,
    new_name: Option<&str>,
    status: StatusCode,
) -> AppResult<Response> {
    let site_name = web::site_name(state).await?;
    let public_categories = links::categories_for(&state.db, access.space.id, None).await?;
    let public_links = links::links_for(&state.db, access.space.id, None).await?;
    let mut counts: HashMap<i64, i64> = HashMap::new();
    for link in &public_links {
        if let Some(category_id) = link.category_id {
            *counts.entry(category_id).or_insert(0) += 1;
        }
    }
    let rows = public_categories
        .iter()
        .map(|category| {
            json!({
                "id": category.id,
                "name": category.name,
                "count": counts.get(&category.id).copied().unwrap_or(0),
            })
        })
        .collect::<Vec<_>>();

    let mut context = web::admin_context(
        &site_name,
        "空间管理",
        "分类",
        "给公共导航分段，并决定成员看到它们的顺序。",
        "categories",
        &session.user,
        &session.csrf,
        false,
        Some(&access.space),
        flash,
    );
    context["categories"] = json!(rows);
    context["add_error"] = json!(add_error);
    context["new_name"] = json!(new_name.unwrap_or(""));
    let html = web::render_page(state, "admin/categories.html", context)?;
    Ok((status, html).into_response())
}

#[allow(clippy::too_many_arguments)]
async fn render_members(
    state: &AppState,
    session: &SessionUser,
    access: &SpaceAccess,
    flash: Option<Value>,
    add_error: Option<&str>,
    add_username: Option<&str>,
    add_role: Option<&str>,
    status: StatusCode,
) -> AppResult<Response> {
    let site_name = web::site_name(state).await?;
    let members = spaces::members_for_space(&state.db, access.space.id).await?;
    let rows = members
        .iter()
        .map(|member| {
            json!({
                "user_id": member.user_id,
                "username": member.username,
                "display_name": member.display_name,
                "initial": member.display_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into()),
                "role": member.role.as_str(),
                "joined_label": format_date(&member.joined_at),
                "last_login_label": member.last_login_at.as_deref().map(format_date),
                "is_active": member.is_active,
                "is_self": member.user_id == session.user.id,
            })
        })
        .collect::<Vec<_>>();
    let mut context = web::admin_context(
        &site_name,
        "空间管理",
        "成员",
        "成员可以浏览公共导航，并维护自己的个人收藏。",
        "members",
        &session.user,
        &session.csrf,
        false,
        Some(&access.space),
        flash,
    );
    context["members"] = json!(rows);
    context["add_error"] = json!(add_error);
    context["add_username"] = json!(add_username.unwrap_or(""));
    context["add_role"] = json!(add_role.unwrap_or("member"));
    let html = web::render_page(state, "admin/members.html", context)?;
    Ok((status, html).into_response())
}

async fn render_settings(
    state: &AppState,
    session: &SessionUser,
    access: &SpaceAccess,
    flash: Option<Value>,
    status: StatusCode,
) -> AppResult<Response> {
    let site_name = web::site_name(state).await?;
    let members = spaces::members_for_space(&state.db, access.space.id).await?;
    let public_links = links::links_for(&state.db, access.space.id, None).await?;
    let public_url = match &state.config.base_url {
        Some(base) => format!("{base}/s/{}", access.space.slug),
        None => format!("/s/{}", access.space.slug),
    };
    let mut context = web::admin_context(
        &site_name,
        "空间管理",
        "空间设置",
        "更新成员看到的名称与简介。",
        "settings",
        &session.user,
        &session.csrf,
        false,
        Some(&access.space),
        flash,
    );
    context["public_url"] = json!(public_url);
    context["created_label"] = json!(format_date(&access.space.created_at));
    context["member_count"] = json!(members.len());
    context["public_links"] = json!(public_links.len());
    let html = web::render_page(state, "admin/settings.html", context)?;
    Ok((status, html).into_response())
}

async fn parse_link_form(
    form: &LinkForm,
    state: &AppState,
    access: &SpaceAccess,
) -> Result<(String, String, String, Option<i64>), String> {
    let title = validate_link_title(&form.title)?;
    let url = normalize_url(&form.url)?;
    let description = validate_description(&form.description)?;
    let category_id = if form.category_id.trim().is_empty() {
        None
    } else {
        form.category_id
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| "分类选择无效。")?
    };
    if let Some(category_id) = category_id {
        let categories = links::categories_for(&state.db, access.space.id, None)
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "failed to load categories");
                "分类加载失败。"
            })?;
        if !categories.iter().any(|category| category.id == category_id) {
            return Err("分类不存在。".to_string());
        }
    }
    Ok((title, url, description, category_id))
}

fn dialog_new(access: &SpaceAccess) -> Value {
    json!({
        "open": true,
        "mode": "new",
        "title": "新增链接",
        "action": format!("/s/{}/admin/links", access.space.slug),
        "error": Value::Null,
        "values": {
            "title": "",
            "url": "",
            "description": "",
            "category_id": Value::Null,
        }
    })
}

fn dialog_edit(access: &SpaceAccess, link: &LinkRecord, error: Option<&str>) -> Value {
    json!({
        "open": true,
        "mode": "edit",
        "title": "编辑链接",
        "action": format!("/s/{}/admin/links/{}", access.space.slug, link.id),
        "error": error,
        "values": {
            "title": link.title,
            "url": link.url,
            "description": link.description,
            "category_id": link.category_id,
        }
    })
}

fn dialog_from_form(
    access: &SpaceAccess,
    form: &LinkForm,
    error: Option<&str>,
    link_id: Option<i64>,
) -> Value {
    let category_id = form.category_id.trim().parse::<i64>().ok();
    let action = match link_id {
        Some(id) => format!("/s/{}/admin/links/{id}", access.space.slug),
        None => format!("/s/{}/admin/links", access.space.slug),
    };
    json!({
        "open": true,
        "mode": if link_id.is_some() { "edit" } else { "new" },
        "title": if link_id.is_some() { "编辑链接" } else { "新增链接" },
        "action": action,
        "error": error,
        "values": {
            "title": form.title,
            "url": form.url,
            "description": form.description,
            "category_id": category_id,
        }
    })
}

/// Only for the explicit "刷新图标" button, where the operator waits for the
/// outcome. Saving a link uses `refresh_icon_in_background` instead.
async fn refresh_icon_sync(state: &AppState, link_id: i64) -> Option<String> {
    if !state.config.icon_fetch {
        return None;
    }
    let future = favicon::refresh_for_link(&state.db, &state.http, &state.config, link_id);
    match tokio::time::timeout(Duration::from_secs(10), future).await {
        Ok(Ok(hash)) => hash,
        Ok(Err(error)) => {
            tracing::warn!(link_id, error = %error, "icon refresh failed");
            None
        }
        Err(_) => {
            tracing::info!(link_id, "icon refresh timed out");
            None
        }
    }
}

fn initial_of(title: &str) -> String {
    title
        .chars()
        .find(|c| !c.is_whitespace())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "·".to_string())
}
