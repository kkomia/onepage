use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use axum_extra::extract::cookie::CookieJar;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use crate::auth;
use crate::error::{AppError, AppResult};
use crate::services::{settings, spaces};
use crate::state::AppState;
use crate::util::{
    now_rfc3339, safe_next, validate_display_name, validate_password, validate_site_name,
    validate_slug, validate_space_name, validate_username,
};
use crate::web;

#[derive(Debug, Deserialize)]
pub struct NextQuery {
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
    #[serde(default)]
    next: String,
}

#[derive(Debug, Deserialize)]
pub struct SetupForm {
    site_name: String,
    username: String,
    display_name: String,
    password: String,
    password_confirm: String,
    space_name: String,
    space_slug: String,
}

#[derive(Debug, Deserialize)]
pub struct PasswordQuery {
    required: Option<String>,
    ok: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PasswordForm {
    csrf: String,
    current_password: String,
    new_password: String,
    new_password_confirm: String,
}

pub async fn home(State(state): State<AppState>, jar: CookieJar) -> Response {
    match home_inner(&state, &jar).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn home_inner(state: &AppState, jar: &CookieJar) -> AppResult<Response> {
    if !settings::setup_complete(&state.db).await? {
        return Ok(Redirect::to("/setup").into_response());
    }
    let Some(session) = web::session_optional(state, jar).await else {
        return Ok(Redirect::to("/login").into_response());
    };
    if session.user.must_change_password {
        return Ok(Redirect::to("/account/password?required=1").into_response());
    }

    let memberships = spaces::memberships_for_user(&state.db, session.user.id).await?;
    if let Some(first) = memberships.first() {
        return Ok(Redirect::to(&format!("/s/{}", first.space.slug)).into_response());
    }

    if session.user.is_super_admin {
        let all = spaces::all_spaces_for_switcher(&state.db, &session.user).await?;
        if let Some(first) = all.first() {
            return Ok(Redirect::to(&format!("/s/{}", first.space.slug)).into_response());
        }
        return Ok(Redirect::to("/admin/spaces").into_response());
    }

    let site_name = web::site_name(state).await?;
    let mut context = web::auth_context(&site_name, "no_space");
    context["user"] = web::user_context(&session.user);
    context["csrf"] = json!(session.csrf);
    let html = web::render_page(state, "no_space.html", context)?;
    Ok(html.into_response())
}

pub async fn login_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<NextQuery>,
) -> Response {
    match login_page_inner(&state, &jar, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn login_page_inner(
    state: &AppState,
    jar: &CookieJar,
    query: NextQuery,
) -> AppResult<Response> {
    if !settings::setup_complete(&state.db).await? {
        return Ok(Redirect::to("/setup").into_response());
    }
    let next = safe_next(query.next.as_deref());

    if let Some(session) = web::session_optional(state, jar).await {
        let target = if session.user.must_change_password {
            "/account/password?required=1".to_string()
        } else {
            next
        };
        return Ok(Redirect::to(&target).into_response());
    }

    let site_name = web::site_name(state).await?;
    render_login(state, &site_name, &next, "", None, StatusCode::OK)
}

pub async fn login_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    match login_submit_inner(state, jar, peer, headers, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn login_submit_inner(
    state: AppState,
    jar: CookieJar,
    peer: SocketAddr,
    headers: HeaderMap,
    form: LoginForm,
) -> AppResult<Response> {
    if !settings::setup_complete(&state.db).await? {
        return Ok(Redirect::to("/setup").into_response());
    }

    let site_name = web::site_name(&state).await?;
    let next = safe_next(Some(form.next.as_str()));
    let username_input = form.username.trim().to_string();
    let ip = peer.ip().to_string();
    let throttle_key = format!("{}|{}", username_input.to_ascii_lowercase(), ip);

    if let Some(seconds) = state.throttle.blocked_for(&throttle_key) {
        let error = format!("尝试次数过多，请在 {seconds} 秒后再试。");
        return render_login(
            &state,
            &site_name,
            &next,
            &username_input,
            Some(&error),
            StatusCode::TOO_MANY_REQUESTS,
        );
    }

    let found = auth::user_with_password(&state.db, &username_input).await?;
    let Some((user, password_hash)) = found else {
        state.throttle.record_failure(&throttle_key);
        return render_login(
            &state,
            &site_name,
            &next,
            &username_input,
            Some("账号或密码不正确。"),
            StatusCode::UNAUTHORIZED,
        );
    };

    if !auth::verify_password(&password_hash, &form.password).await {
        state.throttle.record_failure(&throttle_key);
        return render_login(
            &state,
            &site_name,
            &next,
            &username_input,
            Some("账号或密码不正确。"),
            StatusCode::UNAUTHORIZED,
        );
    }

    if !user.is_active {
        return render_login(
            &state,
            &site_name,
            &next,
            &username_input,
            Some("账号已停用，请联系管理员。"),
            StatusCode::FORBIDDEN,
        );
    }

    state.throttle.clear(&throttle_key);
    auth::mark_login(&state.db, user.id).await?;

    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    let (token, _session) = auth::create_session(
        &state.db,
        user.id,
        state.config.session_days,
        ip,
        user_agent,
    )
    .await?;
    let cookie = auth::session_cookie(&token, &state.config);
    let target = if user.must_change_password {
        "/account/password?required=1".to_string()
    } else {
        next
    };
    Ok((jar.add(cookie), Redirect::to(&target)).into_response())
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar, Form(form): Form<CsrfForm>) -> Response {
    match logout_inner(&state, jar, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn logout_inner(state: &AppState, jar: CookieJar, form: CsrfForm) -> AppResult<Response> {
    if let Some(session) = web::session_optional(state, &jar).await {
        web::verify_page_csrf(&session, &form.csrf)?;
        auth::destroy_session(&state.db, &session.token_hash).await?;
    }
    let jar = jar.add(auth::clear_session_cookie());
    Ok((jar, Redirect::to("/login")).into_response())
}

#[derive(Debug, Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

pub async fn setup_page(State(state): State<AppState>) -> Response {
    match setup_page_inner(&state).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn setup_page_inner(state: &AppState) -> AppResult<Response> {
    if settings::setup_complete(&state.db).await? {
        return Ok(Redirect::to("/login").into_response());
    }
    render_setup(
        state,
        "OnePage",
        SetupValues::default(),
        None,
        StatusCode::OK,
    )
}

pub async fn setup_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<SetupForm>,
) -> Response {
    match setup_submit_inner(state, jar, peer, headers, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn setup_submit_inner(
    state: AppState,
    jar: CookieJar,
    peer: SocketAddr,
    headers: HeaderMap,
    form: SetupForm,
) -> AppResult<Response> {
    if settings::setup_complete(&state.db).await? {
        return Ok(Redirect::to("/login").into_response());
    }

    let values = SetupValues {
        site_name: form.site_name.trim().to_string(),
        username: form.username.trim().to_string(),
        display_name: form.display_name.trim().to_string(),
        space_name: form.space_name.trim().to_string(),
        space_slug: form.space_slug.trim().to_ascii_lowercase(),
    };
    let display_site_name = if values.site_name.is_empty() {
        "OnePage".to_string()
    } else {
        values.site_name.clone()
    };

    let validation = (|| -> Result<(String, String, String, String, String), String> {
        let site_name = validate_site_name(&values.site_name)?;
        let username = validate_username(&values.username)?;
        let display_name = validate_display_name(&values.display_name)?;
        validate_password(&form.password)?;
        if form.password != form.password_confirm {
            return Err("两次输入的密码不一致。".into());
        }
        let space_name = validate_space_name(&values.space_name)?;
        let space_slug = validate_slug(&values.space_slug)?;
        Ok((site_name, username, display_name, space_name, space_slug))
    })();
    let (site_name, username, display_name, space_name, space_slug) = match validation {
        Ok(values) => values,
        Err(error) => {
            return render_setup(
                &state,
                &display_site_name,
                values,
                Some(&error),
                StatusCode::UNPROCESSABLE_ENTITY,
            )
        }
    };

    let password_hash = auth::hash_password(&form.password).await?;
    let now = now_rfc3339();
    let created = state
        .db
        .call(move |conn| {
            let tx = conn.transaction()?;
            let user_count: i64 =
                tx.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
            if user_count > 0 {
                anyhow::bail!("站点已经完成初始化。");
            }
            let username_taken: Option<i64> = tx
                .query_row(
                    "SELECT id FROM users WHERE username = ?1",
                    params![username],
                    |row| row.get(0),
                )
                .ok();
            if username_taken.is_some() {
                anyhow::bail!("这个登录账号已经被使用。");
            }
            let slug_taken: Option<i64> = tx
                .query_row(
                    "SELECT id FROM spaces WHERE slug = ?1",
                    params![space_slug],
                    |row| row.get(0),
                )
                .ok();
            if slug_taken.is_some() {
                anyhow::bail!("这个空间标识已经被使用。");
            }
            tx.execute(
                "INSERT INTO settings (key, value) VALUES ('site_name', ?1)",
                params![site_name],
            )?;
            tx.execute(
                "INSERT INTO users
                    (username, display_name, password_hash, is_super_admin, is_active,
                     must_change_password, created_at)
                 VALUES (?1, ?2, ?3, 1, 1, 0, ?4)",
                params![username, display_name, password_hash, now],
            )?;
            let user_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO spaces (slug, name, description, created_by, created_at)
                 VALUES (?1, ?2, '', ?3, ?4)",
                params![space_slug, space_name, user_id, now],
            )?;
            let space_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memberships (space_id, user_id, role, created_at)
                 VALUES (?1, ?2, 'admin', ?3)",
                params![space_id, user_id, now],
            )?;
            tx.commit()?;
            Ok((user_id, space_slug))
        })
        .await;

    let (user_id, space_slug) = match created {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(error = %error, "setup failed");
            let message = if error.to_string().contains("已经") {
                error.to_string()
            } else {
                "初始化没有完成，请稍后重试。".to_string()
            };
            return render_setup(
                &state,
                &display_site_name,
                values,
                Some(&message),
                StatusCode::INTERNAL_SERVER_ERROR,
            );
        }
    };

    auth::mark_login(&state.db, user_id).await?;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();
    let (token, _) = auth::create_session(
        &state.db,
        user_id,
        state.config.session_days,
        peer.ip().to_string(),
        user_agent,
    )
    .await?;
    let cookie = auth::session_cookie(&token, &state.config);
    Ok((
        jar.add(cookie),
        Redirect::to(&format!("/s/{space_slug}")),
    )
        .into_response())
}

#[derive(Default, Clone)]
struct SetupValues {
    site_name: String,
    username: String,
    display_name: String,
    space_name: String,
    space_slug: String,
}

pub async fn password_page(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<PasswordQuery>,
) -> Response {
    match password_page_inner(&state, &jar, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn password_page_inner(
    state: &AppState,
    jar: &CookieJar,
    query: PasswordQuery,
) -> AppResult<Response> {
    let session = web::require_page_session(state, jar, "/account/password").await?;
    let site_name = web::site_name(state).await?;
    let success = if query.ok.as_deref() == Some("changed") {
        Some("密码已更新。")
    } else {
        None
    };
    render_password(
        state,
        &site_name,
        &session,
        session.user.must_change_password || query.required.is_some(),
        None,
        success,
        StatusCode::OK,
    )
}

pub async fn password_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<PasswordForm>,
) -> Response {
    match password_submit_inner(&state, &jar, form).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn password_submit_inner(
    state: &AppState,
    jar: &CookieJar,
    form: PasswordForm,
) -> AppResult<Response> {
    let session = web::require_page_session(state, jar, "/account/password").await?;
    let site_name = web::site_name(state).await?;
    let required = session.user.must_change_password;
    if !auth::verify_csrf(&session, &form.csrf) {
        return render_password(
            state,
            &site_name,
            &session,
            required,
            Some("页面已过期，请刷新后重试。"),
            None,
            StatusCode::BAD_REQUEST,
        );
    }

    if let Err(message) = validate_password(&form.new_password) {
        return render_password(
            state,
            &site_name,
            &session,
            required,
            Some(&message),
            None,
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }
    if form.new_password != form.new_password_confirm {
        return render_password(
            state,
            &site_name,
            &session,
            required,
            Some("两次输入的新密码不一致。"),
            None,
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }
    if form.new_password == form.current_password {
        return render_password(
            state,
            &site_name,
            &session,
            required,
            Some("新密码不能与当前密码相同。"),
            None,
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    let found = auth::user_with_password(&state.db, &session.user.username).await?;
    let Some((_, current_hash)) = found else {
        return Err(AppError::Internal(anyhow::anyhow!("session user missing")));
    };
    if !auth::verify_password(&current_hash, &form.current_password).await {
        return render_password(
            state,
            &site_name,
            &session,
            required,
            Some("当前密码不正确。"),
            None,
            StatusCode::UNAUTHORIZED,
        );
    }

    let password_hash = auth::hash_password(&form.new_password).await?;
    auth::update_password(
        &state.db,
        session.user.id,
        password_hash,
        Some(session.token_hash.clone()),
    )
    .await?;

    if required {
        return Ok(Redirect::to("/").into_response());
    }
    let session = web::session_optional(state, jar)
        .await
        .ok_or_else(|| AppError::Redirect("/login".into()))?;
    render_password(
        state,
        &site_name,
        &session,
        false,
        None,
        Some("密码已更新，其他设备上的登录已失效。"),
        StatusCode::OK,
    )
}

fn render_login(
    state: &AppState,
    site_name: &str,
    next: &str,
    username: &str,
    error: Option<&str>,
    status: StatusCode,
) -> AppResult<Response> {
    let mut context = web::auth_context(site_name, "login");
    context["next"] = json!(next);
    context["username"] = json!(username);
    context["error"] = json!(error);
    let html = web::render_page(state, "login.html", context)?;
    Ok((status, html).into_response())
}

fn render_setup(
    state: &AppState,
    site_name: &str,
    values: SetupValues,
    error: Option<&str>,
    status: StatusCode,
) -> AppResult<Response> {
    let mut context = web::auth_context(site_name, "setup");
    context["site_name_value"] = json!(values.site_name);
    context["username"] = json!(values.username);
    context["display_name"] = json!(values.display_name);
    context["space_name"] = json!(values.space_name);
    context["space_slug"] = json!(values.space_slug);
    context["error"] = json!(error);
    let html = web::render_page(state, "setup.html", context)?;
    Ok((status, html).into_response())
}

fn render_password(
    state: &AppState,
    site_name: &str,
    session: &crate::models::SessionUser,
    required: bool,
    error: Option<&str>,
    success: Option<&str>,
    status: StatusCode,
) -> AppResult<Response> {
    let mut context = web::auth_context(site_name, "password");
    context["user"] = web::user_context(&session.user);
    context["csrf"] = json!(session.csrf);
    context["required"] = json!(required);
    context["error"] = json!(error);
    context["success"] = json!(success);
    let html = web::render_page(state, "password.html", context)?;
    Ok((status, html).into_response())
}
