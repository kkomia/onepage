use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::access;
use crate::auth;
use crate::error::{AppError, AppResult};
use crate::services::{settings, spaces, users};
use crate::state::AppState;
use crate::util::{
    format_date, validate_description, validate_display_name, validate_password,
    validate_site_name, validate_slug, validate_space_name, validate_username,
};
use crate::web;

#[derive(Debug, Deserialize, Default)]
pub struct SpacesQuery {
    dialog: Option<String>,
    edit: Option<i64>,
    ok: Option<String>,
    err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SpaceForm {
    csrf: String,
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    admin_username: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct UsersQuery {
    dialog: Option<String>,
    reset: Option<i64>,
    ok: Option<String>,
    err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserCreateForm {
    csrf: String,
    username: String,
    display_name: String,
    password: String,
    password_confirm: String,
    #[serde(default)]
    must_change: String,
}

#[derive(Debug, Deserialize)]
pub struct UserResetForm {
    csrf: String,
    password: String,
    password_confirm: String,
}

#[derive(Debug, Deserialize)]
pub struct ToggleForm {
    csrf: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SiteQuery {
    ok: Option<String>,
    err: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SiteForm {
    csrf: String,
    site_name: String,
}

pub async fn index() -> Redirect {
    Redirect::to("/admin/spaces")
}

pub async fn spaces_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<SpacesQuery>,
) -> Response {
    match spaces_page_inner(state, jar, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn spaces_page_inner(
    state: AppState,
    jar: CookieJar,
    query: SpacesQuery,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/spaces").await?;
    access::require_super_admin_page(&session.user)?;
    let flash = web::flash_from_query(query.ok.as_deref(), query.err.as_deref());

    let mut dialog = None;
    if query.dialog.as_deref() == Some("new") {
        dialog = Some(dialog_space_new());
    } else if query.dialog.as_deref() == Some("edit") {
        if let Some(space_id) = query.edit {
            if let Some(space) = spaces::find_space_by_id(&state.db, space_id).await? {
                dialog = Some(dialog_space_edit(&space));
            }
        }
    }
    render_spaces(&state, &session, flash, dialog, StatusCode::OK).await
}

pub async fn create_space(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<SpaceForm>,
) -> Response {
    match create_space_inner(state, jar, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn create_space_inner(
    state: AppState,
    jar: CookieJar,
    form: SpaceForm,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/spaces").await?;
    access::require_super_admin_page(&session.user)?;
    web::verify_page_csrf(&session, &form.csrf)?;

    let parsed = (|| -> Result<(String, String, String), String> {
        let name = validate_space_name(&form.name)?;
        let slug = validate_slug(&form.slug)?;
        let description = validate_description(&form.description)?;
        Ok((name, slug, description))
    })();

    let (name, slug, description) = match parsed {
        Ok(values) => values,
        Err(error) => {
            let values = SpaceFormValues {
                name: form.name.clone(),
                slug: form.slug.clone(),
                description: form.description.clone(),
                admin_username: form.admin_username.clone(),
            };
            return render_spaces(
                &state,
                &session,
                None,
                Some(dialog_space_new_with(&values, Some(&error))),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        }
    };

    let username = form.admin_username.trim().to_string();
    let admin = if username.is_empty() {
        None
    } else {
        users::find_by_username(&state.db, &username).await?
    };
    let Some(admin) = admin else {
        let values = SpaceFormValues {
            name: form.name.clone(),
            slug: form.slug.clone(),
            description: form.description.clone(),
            admin_username: form.admin_username.clone(),
        };
        return render_spaces(
            &state,
            &session,
            None,
            Some(dialog_space_new_with(&values, Some("找不到这个管理员账号。"))),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    };
    if !admin.is_active {
        let values = SpaceFormValues {
            name: form.name.clone(),
            slug: form.slug.clone(),
            description: form.description.clone(),
            admin_username: form.admin_username.clone(),
        };
        return render_spaces(
            &state,
            &session,
            None,
            Some(dialog_space_new_with(&values, Some("这个管理员账号已停用。"))),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }

    match spaces::create_space(
        &state.db,
        &name,
        &slug,
        &description,
        admin.id,
        session.user.id,
    )
    .await
    {
        Ok(_) => Ok(Redirect::to("/admin/spaces?ok=space_created").into_response()),
        Err(error) => {
            let message = error.to_string();
            let display = if message.contains("空间标识") {
                message
            } else {
                tracing::error!(error = %error, "failed to create space");
                "创建空间没有完成，请稍后重试。".to_string()
            };
            let values = SpaceFormValues {
                name: form.name,
                slug: form.slug,
                description: form.description,
                admin_username: form.admin_username,
            };
            render_spaces(
                &state,
                &session,
                None,
                Some(dialog_space_new_with(&values, Some(&display))),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
    }
}

pub async fn update_space(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_id): Path<i64>,
    Form(form): Form<SpaceForm>,
) -> Response {
    match update_space_inner(state, jar, space_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn update_space_inner(
    state: AppState,
    jar: CookieJar,
    space_id: i64,
    form: SpaceForm,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/spaces").await?;
    access::require_super_admin_page(&session.user)?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let Some(space) = spaces::find_space_by_id(&state.db, space_id).await? else {
        return Err(AppError::not_found("空间不存在。"));
    };

    let name = match validate_space_name(&form.name) {
        Ok(name) => name,
        Err(error) => {
            return render_spaces(
                &state,
                &session,
                None,
                Some(dialog_space_edit_with(&space, &form.name, &form.description, Some(&error))),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
    };
    let description = match validate_description(&form.description) {
        Ok(description) => description,
        Err(error) => {
            return render_spaces(
                &state,
                &session,
                None,
                Some(dialog_space_edit_with(&space, &form.name, &form.description, Some(&error))),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
    };
    spaces::update_space(&state.db, space_id, &name, &description).await?;
    Ok(Redirect::to("/admin/spaces?ok=space_updated").into_response())
}

pub async fn users_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<UsersQuery>,
) -> Response {
    match users_page_inner(state, jar, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn users_page_inner(
    state: AppState,
    jar: CookieJar,
    query: UsersQuery,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/users").await?;
    access::require_super_admin_page(&session.user)?;
    let flash = web::flash_from_query(query.ok.as_deref(), query.err.as_deref());

    let mut dialog = None;
    if query.dialog.as_deref() == Some("new") {
        dialog = Some(dialog_user_new(None));
    } else if query.dialog.as_deref() == Some("reset") {
        if let Some(user_id) = query.reset {
            if let Some(user) = auth::user_by_id(&state.db, user_id).await? {
                dialog = Some(dialog_user_reset(&user, None));
            }
        }
    }
    render_users(&state, &session, flash, dialog, StatusCode::OK).await
}

pub async fn create_user(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<UserCreateForm>,
) -> Response {
    match create_user_inner(state, jar, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn create_user_inner(
    state: AppState,
    jar: CookieJar,
    form: UserCreateForm,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/users").await?;
    access::require_super_admin_page(&session.user)?;
    web::verify_page_csrf(&session, &form.csrf)?;

    let parsed = (|| -> Result<(String, String), String> {
        let username = validate_username(&form.username)?;
        let display_name = validate_display_name(&form.display_name)?;
        validate_password(&form.password)?;
        if form.password != form.password_confirm {
            return Err("两次输入的密码不一致。".into());
        }
        Ok((username, display_name))
    })();

    let (username, display_name) = match parsed {
        Ok(values) => values,
        Err(error) => {
            let values = UserCreateValues {
                username: form.username, display_name: form.display_name,
                must_change: form.must_change == "on",
            };
            return render_users(
                &state,
                &session,
                None,
                Some(dialog_user_new_with(Some(&values), Some(&error))),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await;
        }
    };

    let must_change = form.must_change == "on";
    let password_hash = auth::hash_password(&form.password).await?;
    match users::create_user(
        &state.db,
        &username,
        &display_name,
        &password_hash,
        must_change,
    )
    .await
    {
        Ok(_) => Ok(Redirect::to("/admin/users?ok=user_created").into_response()),
        Err(error) => {
            let message = if error.to_string().contains("已经存在") {
                "这个账号已经存在。".to_string()
            } else {
                tracing::error!(error = %error, "failed to create user");
                "创建账号没有完成，请稍后重试。".to_string()
            };
            let values = UserCreateValues {
                username: form.username,
                display_name: form.display_name,
                must_change: form.must_change == "on",
            };
            render_users(
                &state,
                &session,
                None,
                Some(dialog_user_new_with(Some(&values), Some(&message))),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
    }
}

pub async fn toggle_user(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(user_id): Path<i64>,
    Form(form): Form<ToggleForm>,
) -> Response {
    match toggle_user_inner(state, jar, user_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn toggle_user_inner(
    state: AppState,
    jar: CookieJar,
    user_id: i64,
    form: ToggleForm,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/users").await?;
    access::require_super_admin_page(&session.user)?;
    web::verify_page_csrf(&session, &form.csrf)?;
    if user_id == session.user.id {
        return Ok(Redirect::to("/admin/users?err=user_self").into_response());
    }
    let Some(target) = auth::user_by_id(&state.db, user_id).await? else {
        return Ok(Redirect::to("/admin/users?err=user_missing").into_response());
    };
    if target.is_super_admin && target.is_active {
        let count = users::active_super_admin_count(&state.db).await?;
        if count <= 1 {
            return Ok(Redirect::to("/admin/users?err=last_super_admin").into_response());
        }
    }
    users::set_active(&state.db, user_id, !target.is_active).await?;
    let code = if target.is_active {
        "user_disabled"
    } else {
        "user_enabled"
    };
    Ok(Redirect::to(&format!("/admin/users?ok={code}")).into_response())
}

pub async fn reset_user_password(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(user_id): Path<i64>,
    Form(form): Form<UserResetForm>,
) -> Response {
    match reset_user_password_inner(state, jar, user_id, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn reset_user_password_inner(
    state: AppState,
    jar: CookieJar,
    user_id: i64,
    form: UserResetForm,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/users").await?;
    access::require_super_admin_page(&session.user)?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let Some(target) = auth::user_by_id(&state.db, user_id).await? else {
        return Ok(Redirect::to("/admin/users?err=user_missing").into_response());
    };

    let validation = (|| -> Result<(), String> {
        validate_password(&form.password)?;
        if form.password != form.password_confirm {
            return Err("两次输入的密码不一致。".into());
        }
        Ok(())
    })();
    if let Err(error) = validation {
        return render_users(
            &state,
            &session,
            None,
            Some(dialog_user_reset_with(&target, Some(&error))),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }

    let password_hash = auth::hash_password(&form.password).await?;
    auth::reset_password(&state.db, user_id, password_hash).await?;
    Ok(Redirect::to("/admin/users?ok=password_reset").into_response())
}

pub async fn site_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<SiteQuery>,
) -> Response {
    match site_page_inner(state, jar, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn site_page_inner(
    state: AppState,
    jar: CookieJar,
    query: SiteQuery,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/site").await?;
    access::require_super_admin_page(&session.user)?;
    let flash = web::flash_from_query(query.ok.as_deref(), query.err.as_deref());
    render_site(&state, &session, flash, StatusCode::OK).await
}

pub async fn update_site(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<SiteForm>,
) -> Response {
    match update_site_inner(state, jar, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn update_site_inner(
    state: AppState,
    jar: CookieJar,
    form: SiteForm,
) -> AppResult<Response> {
    let session = web::require_ready_page_session(&state, &jar, "/admin/site").await?;
    access::require_super_admin_page(&session.user)?;
    web::verify_page_csrf(&session, &form.csrf)?;
    let site_name = match validate_site_name(&form.site_name) {
        Ok(name) => name,
        Err(_) => return Ok(Redirect::to("/admin/site?err=save_failed").into_response()),
    };
    settings::set_setting(&state.db, "site_name", &site_name).await?;
    Ok(Redirect::to("/admin/site?ok=site_updated").into_response())
}

async fn render_spaces(
    state: &AppState,
    session: &crate::models::SessionUser,
    flash: Option<Value>,
    dialog: Option<Value>,
    status: StatusCode,
) -> AppResult<Response> {
    let site_name = web::site_name(state).await?;
    let summaries = spaces::summaries(&state.db).await?;
    let choices = users::user_choices(&state.db).await?;
    let all_users = users::list_users(&state.db).await?;

    let stats = json!({
        "spaces": summaries.len(),
        "users": all_users.len(),
        "public_links": summaries.iter().map(|item| item.public_links).sum::<i64>(),
        "private_links": summaries.iter().map(|item| item.private_links).sum::<i64>(),
    });
    let rows = summaries
        .iter()
        .map(|item| {
            json!({
                "space": {
                    "id": item.space.id,
                    "slug": item.space.slug,
                    "name": item.space.name,
                    "description": item.space.description,
                    "created_at": item.space.created_at,
                },
                "initial": item.space.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "·".into()),
                "member_count": item.member_count,
                "public_links": item.public_links,
                "private_links": item.private_links,
                "admins": item.admins,
            })
        })
        .collect::<Vec<_>>();
    let choices_json = choices
        .iter()
        .map(|choice| {
            json!({
                "id": choice.id,
                "username": choice.username,
                "display_name": choice.display_name,
            })
        })
        .collect::<Vec<_>>();

    let mut context = web::admin_context(
        &site_name,
        "系统管理",
        "空间",
        "每个空间拥有独立的成员、公共导航和个人收藏。",
        "spaces",
        &session.user,
        &session.csrf,
        true,
        None,
        flash,
    );
    context["summaries"] = json!(rows);
    context["stats"] = stats;
    context["user_choices"] = json!(choices_json);
    context["dialog"] = dialog.unwrap_or(Value::Null);
    let html = web::render_page(state, "admin/spaces.html", context)?;
    Ok((status, html).into_response())
}

async fn render_users(
    state: &AppState,
    session: &crate::models::SessionUser,
    flash: Option<Value>,
    dialog: Option<Value>,
    status: StatusCode,
) -> AppResult<Response> {
    let site_name = web::site_name(state).await?;
    let all_users = users::list_users(&state.db).await?;
    let rows = all_users
        .iter()
        .map(|item| {
            json!({
                "user": {
                    "id": item.user.id,
                    "username": item.user.username,
                    "display_name": item.user.display_name,
                    "is_super_admin": item.user.is_super_admin,
                    "is_active": item.user.is_active,
                    "must_change_password": item.user.must_change_password,
                    "last_login_at": item.user.last_login_at,
                },
                "initial": item.user.display_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".into()),
                "last_login_label": item.user.last_login_at.as_deref().map(format_date),
                "created_at_label": format_date(&item.created_at),
                "spaces": item.spaces,
            })
        })
        .collect::<Vec<_>>();
    let mut context = web::admin_context(
        &site_name,
        "系统管理",
        "用户",
        "创建、停用账号，或重置忘记的密码。",
        "users",
        &session.user,
        &session.csrf,
        true,
        None,
        flash,
    );
    context["users"] = json!(rows);
    context["current_user_id"] = json!(session.user.id);
    context["dialog"] = dialog.unwrap_or(Value::Null);
    let html = web::render_page(state, "admin/users.html", context)?;
    Ok((status, html).into_response())
}

async fn render_site(
    state: &AppState,
    session: &crate::models::SessionUser,
    flash: Option<Value>,
    status: StatusCode,
) -> AppResult<Response> {
    let site_name = web::site_name(state).await?;
    let mut context = web::admin_context(
        &site_name,
        "系统管理",
        "站点",
        "站点名称与运行参数。",
        "site",
        &session.user,
        &session.csrf,
        true,
        None,
        flash,
    );
    context["site_name_value"] = json!(site_name);
    context["base_url"] = json!(state.config.base_url);
    context["listen_addr"] = json!(state.config.bind.to_string());
    context["session_days"] = json!(state.config.session_days);
    context["icon_fetch"] = json!(state.config.icon_fetch);
    context["data_dir"] = json!(state.config.data_dir.display().to_string());
    let html = web::render_page(state, "admin/site.html", context)?;
    Ok((status, html).into_response())
}

struct SpaceFormValues {
    name: String,
    slug: String,
    description: String,
    admin_username: String,
}

fn dialog_space_new() -> Value {
    dialog_space_new_with(
        &SpaceFormValues {
            name: String::new(),
            slug: String::new(),
            description: String::new(),
            admin_username: String::new(),
        },
        None,
    )
}

fn dialog_space_new_with(values: &SpaceFormValues, error: Option<&str>) -> Value {
    json!({
        "open": true,
        "mode": "new",
        "action": "/admin/spaces",
        "error": error,
        "values": {
            "name": values.name,
            "slug": values.slug,
            "description": values.description,
            "admin_username": values.admin_username,
        }
    })
}

fn dialog_space_edit(space: &crate::models::Space) -> Value {
    dialog_space_edit_with(space, &space.name, &space.description, None)
}

fn dialog_space_edit_with(
    space: &crate::models::Space,
    name: &str,
    description: &str,
    error: Option<&str>,
) -> Value {
    json!({
        "open": true,
        "mode": "edit",
        "action": format!("/admin/spaces/{}", space.id),
        "error": error,
        "values": {
            "id": space.id,
            "name": name,
            "slug": space.slug,
            "description": description,
        }
    })
}

struct UserCreateValues {
    username: String,
    display_name: String,
    must_change: bool,
}

fn dialog_user_new(error: Option<&str>) -> Value {
    dialog_user_new_with(
        Some(&UserCreateValues {
            username: String::new(),
            display_name: String::new(),
            must_change: true,
        }),
        error,
    )
}

fn dialog_user_new_with(values: Option<&UserCreateValues>, error: Option<&str>) -> Value {
    let values = values.cloned().unwrap_or(UserCreateValues {
        username: String::new(),
        display_name: String::new(),
        must_change: true,
    });
    json!({
        "open": true,
        "mode": "new",
        "action": "/admin/users",
        "error": error,
        "values": {
            "username": values.username,
            "display_name": values.display_name,
            "must_change": values.must_change,
        }
    })
}

fn dialog_user_reset(user: &crate::models::User, error: Option<&str>) -> Value {
    dialog_user_reset_with(user, error)
}

fn dialog_user_reset_with(user: &crate::models::User, error: Option<&str>) -> Value {
    json!({
        "open": true,
        "mode": "reset",
        "action": format!("/admin/users/{}/password", user.id),
        "target_name": user.display_name,
        "error": error,
    })
}

impl Clone for UserCreateValues {
    fn clone(&self) -> Self {
        Self {
            username: self.username.clone(),
            display_name: self.display_name.clone(),
            must_change: self.must_change,
        }
    }
}
