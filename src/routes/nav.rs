use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::json;

use crate::access;
use crate::error::{AppError, AppResult};
use crate::services::{favicon, links, nav, spaces};
use crate::state::AppState;
use crate::web;

#[derive(Debug, Deserialize, Default)]
pub struct NavQuery {
    ok: Option<String>,
    err: Option<String>,
}

pub async fn page(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    Query(query): Query<NavQuery>,
) -> Response {
    match page_inner(state, jar, space_slug, query).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn page_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    query: NavQuery,
) -> AppResult<Response> {
    let next = format!("/s/{space_slug}");
    let session = web::require_ready_page_session(&state, &jar, &next).await?;
    let access = access::require_page_space(&state.db, &session.user, &space_slug).await?;

    let flash = web::flash_from_query(query.ok.as_deref(), query.err.as_deref());
    let bootstrap = nav::build_bootstrap(&state, &session, &access, flash)
        .await
        .map_err(AppError::Internal)?;
    let bootstrap_json = serde_json::to_string(&bootstrap)
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
    let site_name = web::site_name(&state).await?;
    let mut context = web::base_context(&site_name, "nav");
    context["bootstrap_json"] = json!(bootstrap_json);
    let html = web::render_page(&state, "nav.html", context)?;
    Ok(html.into_response())
}

pub async fn go_link(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(link_id): Path<i64>,
) -> Response {
    match go_link_inner(&state, &jar, link_id).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn go_link_inner(state: &AppState, jar: &CookieJar, link_id: i64) -> AppResult<Response> {
    let session =
        web::require_page_session(state, jar, &format!("/go/{link_id}")).await?;
    let Some(link) = links::link_by_id(&state.db, link_id).await? else {
        return Err(AppError::not_found("这个链接已经不存在。"));
    };
    let Some(space) = spaces::find_space_by_id(&state.db, link.space_id).await? else {
        return Err(AppError::not_found("这个链接已经不存在。"));
    };
    let access = access::require_page_space(&state.db, &session.user, &space.slug).await?;

    match link.owner_id {
        Some(owner_id) => {
            if owner_id != session.user.id || !access.is_member() {
                return Err(AppError::not_found("这个链接已经不存在。"));
            }
        }
        None => {
            if access.role.is_none() {
                return Err(AppError::not_found("这个链接已经不存在。"));
            }
        }
    }

    if !link.url.starts_with("http://") && !link.url.starts_with("https://") {
        return Err(AppError::bad_request("链接地址无效。"));
    }

    if let Err(error) = links::record_click(&state.db, link.id, session.user.id).await {
        tracing::warn!(link_id, error = %error, "failed to record click");
    }
    Ok(Redirect::to(&link.url).into_response())
}

pub async fn icon(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(hash): Path<String>,
    request_headers: HeaderMap,
) -> Response {
    match icon_inner(&state, jar, hash, request_headers).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn icon_inner(
    state: &AppState,
    jar: CookieJar,
    hash: String,
    request_headers: HeaderMap,
) -> AppResult<Response> {
    if web::session_optional(state, &jar).await.is_none() {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }
    if hash.len() != 16 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let Some((content_type, data, _fetched_at)) = favicon::icon_by_hash(&state.db, &hash).await?
    else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let etag = format!("\"{hash}\"");
    if request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(|value| value == etag)
        .unwrap_or(false)
    {
        return Ok(StatusCode::NOT_MODIFIED.into_response());
    }

    let mut response = data.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=2592000, immutable"),
    );
    headers.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
    Ok(response)
}
