pub mod admin_global;
pub mod admin_space;
pub mod api;
pub mod assets;
pub mod auth_pages;
pub mod nav;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde_json::json;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let pages = Router::new()
        .route("/", get(auth_pages::home))
        .route("/login", get(auth_pages::login_page).post(auth_pages::login_submit))
        .route("/logout", post(auth_pages::logout))
        .route("/setup", get(auth_pages::setup_page).post(auth_pages::setup_submit))
        .route(
            "/account/password",
            get(auth_pages::password_page).post(auth_pages::password_submit),
        )
        .route("/s/{space}", get(nav::page))
        .route("/go/{id}", get(nav::go_link))
        .route("/icons/{hash}", get(nav::icon))
        .route("/api/s/{space}/links", post(api::create_link))
        .route(
            "/api/s/{space}/links/{id}",
            put(api::update_link).delete(api::delete_link),
        )
        .route(
            "/api/s/{space}/links/{id}/refresh-icon",
            post(api::refresh_icon),
        )
        .route(
            "/api/s/{space}/categories",
            post(api::create_category),
        )
        .route(
            "/api/s/{space}/categories/{id}",
            patch(api::update_category).delete(api::delete_category),
        )
        .route("/s/{space}/admin", get(admin_space::index))
        .route(
            "/s/{space}/admin/links",
            get(admin_space::links_page).post(admin_space::create_link),
        )
        .route(
            "/s/{space}/admin/links/{id}",
            post(admin_space::update_link),
        )
        .route(
            "/s/{space}/admin/links/{id}/delete",
            post(admin_space::delete_link),
        )
        .route(
            "/s/{space}/admin/links/{id}/move",
            post(admin_space::move_link),
        )
        .route(
            "/s/{space}/admin/links/{id}/icon",
            post(admin_space::refresh_link_icon),
        )
        .route(
            "/s/{space}/admin/categories",
            get(admin_space::categories_page).post(admin_space::create_category),
        )
        .route(
            "/s/{space}/admin/categories/{id}",
            post(admin_space::rename_category),
        )
        .route(
            "/s/{space}/admin/categories/{id}/delete",
            post(admin_space::delete_category),
        )
        .route(
            "/s/{space}/admin/categories/{id}/move",
            post(admin_space::move_category),
        )
        .route(
            "/s/{space}/admin/members",
            get(admin_space::members_page).post(admin_space::add_member),
        )
        .route(
            "/s/{space}/admin/members/{user_id}/role",
            post(admin_space::set_role),
        )
        .route(
            "/s/{space}/admin/members/{user_id}/remove",
            post(admin_space::remove_member),
        )
        .route(
            "/s/{space}/admin/settings",
            get(admin_space::settings_page).post(admin_space::update_settings),
        )
        .route("/admin", get(admin_global::index))
        .route(
            "/admin/spaces",
            get(admin_global::spaces_page).post(admin_global::create_space),
        )
        .route("/admin/spaces/{id}", post(admin_global::update_space))
        .route(
            "/admin/users",
            get(admin_global::users_page).post(admin_global::create_user),
        )
        .route("/admin/users/{id}/toggle", post(admin_global::toggle_user))
        .route(
            "/admin/users/{id}/password",
            post(admin_global::reset_user_password),
        )
        .route(
            "/admin/site",
            get(admin_global::site_page).post(admin_global::update_site),
        );

    let assets = Router::new()
        .route("/assets/{*path}", get(assets::serve))
        .route(
            "/favicon.ico",
            get(|| async { axum::response::Redirect::permanent("/assets/favicon.svg") }),
        );

    Router::new()
        .merge(assets)
        .merge(pages)
        .route("/healthz", get(health))
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        // Applies to /assets/* as well, which the page router used to cover alone.
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::max(128 * 1024))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Response {
    if state.db.health().await {
        (StatusCode::OK, "ok").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response()
    }
}

async fn fallback(request: Request) -> Response {
    if request.uri().path().starts_with("/api/") {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "接口不存在" })),
        )
            .into_response()
    } else {
        AppError::not_found("这条路径不存在。").into_response()
    }
}

async fn security_headers(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; \
             img-src 'self' data:; \
             font-src 'self'; \
             style-src 'self' 'unsafe-inline'; \
             script-src 'self'; \
             connect-src 'self'; \
             form-action 'self'; \
             frame-ancestors 'none'; \
             base-uri 'self'; \
             object-src 'none'",
        ),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    if state.config.secure_cookies {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }

    response
}
