use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::access;
use crate::error::{ApiError, ApiResult};
use crate::models::LinkRecord;
use crate::services::{favicon, links};
use crate::state::AppState;
use crate::util::{
    normalize_url, pinyin_index, validate_category_name, validate_description, validate_link_title,
};
use crate::web;

/// Replacing a link: every field is read on each request, so a field that is
/// simply omitted is treated as empty rather than preserved. The route is PUT
/// to make that contract explicit.
#[derive(Debug, Deserialize)]
pub struct LinkPayload {
    title: String,
    url: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    category_id: Option<i64>,
    #[serde(default)]
    new_category: String,
}

#[derive(Debug, Deserialize)]
pub struct CategoryPayload {
    name: String,
}

/// Rejected JSON has to come back in the same shape as every other API error,
/// otherwise the client cannot show the field-level message.
fn payload_or_error<T>(result: Result<Json<T>, JsonRejection>) -> ApiResult<T> {
    result.map(|Json(value)| value).map_err(|error| {
        ApiError::new(StatusCode::BAD_REQUEST, error.body_text())
    })
}

pub async fn create_link(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<LinkPayload>, JsonRejection>,
) -> Response {
    match create_link_inner(state, jar, space_slug, headers, payload).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn create_link_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    headers: HeaderMap,
    payload: Result<Json<LinkPayload>, JsonRejection>,
) -> ApiResult<Response> {
    let payload = payload_or_error(payload)?;
    let session = web::require_api_session(&state, &jar).await?;
    web::verify_api_csrf(&session, &headers)?;
    let access = access::require_api_member(&state.db, &session.user, &space_slug).await?;

    let title = validate_link_title(&payload.title)
        .map_err(|message| ApiError::validation(message, "title"))?;
    let url = normalize_url(&payload.url)
        .map_err(|message| ApiError::validation(message, "url"))?;
    let description = validate_description(&payload.description)
        .map_err(|message| ApiError::validation(message, "description"))?;

    let (category_id, category_name) = resolve_category(
        &state,
        access.space.id,
        session.user.id,
        payload.category_id,
        &payload.new_category,
    )
    .await?;

    let link = links::create_link(
        &state.db,
        links::NewLink {
            space_id: access.space.id,
            owner_id: Some(session.user.id),
            category_id,
            title: &title,
            url: &url,
            description: &description,
            created_by: session.user.id,
        },
    )
    .await
    .map_err(ApiError::internal)?;

    state.refresh_icon_in_background(link.id);

    let category = category_id.map(|id| {
        json!({
            "id": id,
            "name": category_name.clone().unwrap_or_default(),
            "scope": "mine",
        })
    });
    Ok(Json(json!({
        "link": link_json(&link, category_name.as_deref(), "mine"),
        "category": category,
    }))
    .into_response())
}

pub async fn update_link(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, link_id)): Path<(String, i64)>,
    headers: HeaderMap,
    payload: Result<Json<LinkPayload>, JsonRejection>,
) -> Response {
    match update_link_inner(state, jar, space_slug, link_id, headers, payload).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn update_link_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    link_id: i64,
    headers: HeaderMap,
    payload: Result<Json<LinkPayload>, JsonRejection>,
) -> ApiResult<Response> {
    let payload = payload_or_error(payload)?;
    let session = web::require_api_session(&state, &jar).await?;
    web::verify_api_csrf(&session, &headers)?;
    let access = access::require_api_member(&state.db, &session.user, &space_slug).await?;

    let Some(existing) = links::link_by_id(&state.db, link_id).await.map_err(ApiError::internal)?
    else {
        return Err(ApiError::not_found());
    };
    if existing.space_id != access.space.id || existing.owner_id != Some(session.user.id) {
        return Err(ApiError::not_found());
    }

    let title = validate_link_title(&payload.title)
        .map_err(|message| ApiError::validation(message, "title"))?;
    let url = normalize_url(&payload.url)
        .map_err(|message| ApiError::validation(message, "url"))?;
    let description = validate_description(&payload.description)
        .map_err(|message| ApiError::validation(message, "description"))?;

    let (category_id, category_name) = resolve_category(
        &state,
        access.space.id,
        session.user.id,
        payload.category_id,
        &payload.new_category,
    )
    .await?;

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
    .await
    .map_err(ApiError::internal)?;

    if url_changed || updated.icon_hash.is_none() {
        state.refresh_icon_in_background(updated.id);
    }

    let category = category_id.map(|id| {
        json!({
            "id": id,
            "name": category_name.clone().unwrap_or_default(),
            "scope": "mine",
        })
    });
    Ok(Json(json!({
        "link": link_json(&updated, category_name.as_deref(), "mine"),
        "category": category,
    }))
    .into_response())
}

pub async fn delete_link(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, link_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Response {
    match delete_link_inner(state, jar, space_slug, link_id, headers).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn delete_link_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    link_id: i64,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let session = web::require_api_session(&state, &jar).await?;
    web::verify_api_csrf(&session, &headers)?;
    let access = access::require_api_member(&state.db, &session.user, &space_slug).await?;

    let Some(existing) = links::link_by_id(&state.db, link_id).await.map_err(ApiError::internal)?
    else {
        return Err(ApiError::not_found());
    };
    if existing.space_id != access.space.id || existing.owner_id != Some(session.user.id) {
        return Err(ApiError::not_found());
    }
    links::delete_link(&state.db, link_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })).into_response())
}

pub async fn refresh_icon(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, link_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Response {
    match refresh_icon_inner(state, jar, space_slug, link_id, headers).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn refresh_icon_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    link_id: i64,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let session = web::require_api_session(&state, &jar).await?;
    web::verify_api_csrf(&session, &headers)?;
    let access = access::require_api_member(&state.db, &session.user, &space_slug).await?;
    let Some(existing) = links::link_by_id(&state.db, link_id).await.map_err(ApiError::internal)?
    else {
        return Err(ApiError::not_found());
    };
    if existing.space_id != access.space.id || existing.owner_id != Some(session.user.id) {
        return Err(ApiError::not_found());
    }

    let fetch = favicon::refresh_for_link(&state.db, &state.http, &state.config, link_id);
    let icon_hash = match tokio::time::timeout(Duration::from_secs(10), fetch).await {
        Ok(Ok(hash)) => hash,
        Ok(Err(error)) => {
            tracing::warn!(link_id, error = %error, "manual icon refresh failed");
            None
        }
        Err(_) => {
            tracing::info!(link_id, "manual icon refresh timed out");
            None
        }
    };
    Ok(Json(json!({
        "iconUrl": icon_hash.map(|hash| format!("/icons/{hash}")),
    }))
    .into_response())
}

pub async fn create_category(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(space_slug): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<CategoryPayload>, JsonRejection>,
) -> Response {
    match create_category_inner(state, jar, space_slug, headers, payload).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn create_category_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    headers: HeaderMap,
    payload: Result<Json<CategoryPayload>, JsonRejection>,
) -> ApiResult<Response> {
    let payload = payload_or_error(payload)?;
    let session = web::require_api_session(&state, &jar).await?;
    web::verify_api_csrf(&session, &headers)?;
    let access = access::require_api_member(&state.db, &session.user, &space_slug).await?;
    let name = validate_category_name(&payload.name)
        .map_err(|message| ApiError::validation(message, "name"))?;

    let existing = links::categories_for(&state.db, access.space.id, Some(session.user.id))
        .await
        .map_err(ApiError::internal)?;
    if existing
        .iter()
        .any(|category| category.name.eq_ignore_ascii_case(&name))
    {
        return Err(ApiError::validation("这个分类已经存在。", "name"));
    }

    let category = links::create_category(
        &state.db,
        access.space.id,
        Some(session.user.id),
        &name,
    )
    .await
    .map_err(ApiError::internal)?;
    Ok(Json(json!({
        "category": {
            "id": category.id,
            "name": category.name,
            "scope": "mine",
            "count": 0,
        }
    }))
    .into_response())
}

pub async fn update_category(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, category_id)): Path<(String, i64)>,
    headers: HeaderMap,
    payload: Result<Json<CategoryPayload>, JsonRejection>,
) -> Response {
    match update_category_inner(state, jar, space_slug, category_id, headers, payload).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn update_category_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    category_id: i64,
    headers: HeaderMap,
    payload: Result<Json<CategoryPayload>, JsonRejection>,
) -> ApiResult<Response> {
    let payload = payload_or_error(payload)?;
    let session = web::require_api_session(&state, &jar).await?;
    web::verify_api_csrf(&session, &headers)?;
    let access = access::require_api_member(&state.db, &session.user, &space_slug).await?;
    let name = validate_category_name(&payload.name)
        .map_err(|message| ApiError::validation(message, "name"))?;

    let Some(category) = links::category_by_id(&state.db, category_id)
        .await
        .map_err(ApiError::internal)?
    else {
        return Err(ApiError::not_found());
    };
    if category.space_id != access.space.id || category.owner_id != Some(session.user.id) {
        return Err(ApiError::not_found());
    }

    let categories = links::categories_for(&state.db, access.space.id, Some(session.user.id))
        .await
        .map_err(ApiError::internal)?;
    if categories
        .iter()
        .any(|other| other.id != category_id && other.name.eq_ignore_ascii_case(&name))
    {
        return Err(ApiError::validation("这个分类已经存在。", "name"));
    }

    links::rename_category(&state.db, category_id, &name)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true, "name": name })).into_response())
}

pub async fn delete_category(
    State(state): State<AppState>,
    jar: CookieJar,
    Path((space_slug, category_id)): Path<(String, i64)>,
    headers: HeaderMap,
) -> Response {
    match delete_category_inner(state, jar, space_slug, category_id, headers).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn delete_category_inner(
    state: AppState,
    jar: CookieJar,
    space_slug: String,
    category_id: i64,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let session = web::require_api_session(&state, &jar).await?;
    web::verify_api_csrf(&session, &headers)?;
    let access = access::require_api_member(&state.db, &session.user, &space_slug).await?;
    let Some(category) = links::category_by_id(&state.db, category_id)
        .await
        .map_err(ApiError::internal)?
    else {
        return Err(ApiError::not_found());
    };
    if category.space_id != access.space.id || category.owner_id != Some(session.user.id) {
        return Err(ApiError::not_found());
    }
    links::delete_category(&state.db, category_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })).into_response())
}

async fn resolve_category(
    state: &AppState,
    space_id: i64,
    user_id: i64,
    category_id: Option<i64>,
    new_category: &str,
) -> ApiResult<(Option<i64>, Option<String>)> {
    let new_name = new_category.trim();
    let categories = links::categories_for(&state.db, space_id, Some(user_id))
        .await
        .map_err(ApiError::internal)?;

    if !new_name.is_empty() {
        let name = validate_category_name(new_name)
            .map_err(|message| ApiError::validation(message, "new_category"))?;
        if let Some(existing) = categories
            .iter()
            .find(|category| category.name.eq_ignore_ascii_case(&name))
        {
            return Ok((Some(existing.id), Some(existing.name.clone())));
        }
        let category = links::create_category(&state.db, space_id, Some(user_id), &name)
            .await
            .map_err(ApiError::internal)?;
        return Ok((Some(category.id), Some(category.name)));
    }

    if let Some(category_id) = category_id {
        let Some(category) = categories.iter().find(|category| category.id == category_id) else {
            return Err(ApiError::validation("分类不存在。", "category_id"));
        };
        return Ok((Some(category.id), Some(category.name.clone())));
    }

    Ok((None, None))
}

fn link_json(link: &LinkRecord, category_name: Option<&str>, scope: &str) -> Value {
    let (py, pyi) = pinyin_index(&format!(
        "{} {}",
        link.title,
        category_name.unwrap_or("")
    ));
    json!({
        "id": link.id,
        "title": link.title,
        "url": link.url,
        "description": link.description,
        "categoryId": link.category_id,
        "scope": scope,
        "iconUrl": link.icon_hash.as_ref().map(|hash| format!("/icons/{hash}")),
        "clickCount": link.click_count,
        "createdAt": link.created_at,
        "py": py,
        "pyi": pyi,
    })
}
