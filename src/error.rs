use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("redirect")]
    Redirect(String),
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0}")]
    BadRequest(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn not_found(message: &'static str) -> Self {
        Self::NotFound(message)
    }

    pub fn forbidden(message: &'static str) -> Self {
        Self::Forbidden(message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Redirect(location) => {
                let mut response = StatusCode::SEE_OTHER.into_response();
                if let Ok(value) = location.parse() {
                    response.headers_mut().insert(header::LOCATION, value);
                }
                response
            }
            AppError::NotFound(message) => error_page(StatusCode::NOT_FOUND, "页面不在这里", message),
            AppError::Forbidden(message) => {
                error_page(StatusCode::FORBIDDEN, "暂时无法访问", message)
            }
            AppError::BadRequest(message) => {
                error_page(StatusCode::BAD_REQUEST, "请求没有完成", &message)
            }
            AppError::Internal(error) => {
                tracing::error!(error = ?error, "request failed");
                error_page(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "服务器开小差了",
                    "这次请求没有完成，请稍后再试。",
                )
            }
        }
    }
}

fn error_page(status: StatusCode, title: &str, message: &str) -> Response {
    let body = format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>{title} · OnePage</title>
<link rel="icon" href="/assets/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/assets/fonts.css?v={version}">
<link rel="stylesheet" href="/assets/app.css?v={version}">
<script src="/assets/theme.js?v={version}"></script>
</head>
<body class="error-page">
<main class="error-card">
  <div class="error-mark" aria-hidden="true"><span></span></div>
  <p class="error-code">{code}</p>
  <h1>{title}</h1>
  <p class="error-message">{message}</p>
  <a class="btn btn-primary" href="/">回到导航</a>
</main>
</body>
</html>"#,
        title = escape_html(title),
        message = escape_html(message),
        code = status.as_u16(),
        version = crate::templates::ASSET_VERSION,
    );
    (status, Html(body)).into_response()
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub field: Option<&'static str>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            field: None,
        }
    }

    pub fn field(mut self, field: &'static str) -> Self {
        self.field = Some(field);
        self
    }

    pub fn validation(message: impl Into<String>, field: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message).field(field)
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "登录已过期，请刷新页面重新登录")
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "没有权限执行这个操作")
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "内容不存在")
    }

    pub fn internal(error: anyhow::Error) -> Self {
        tracing::error!(error = ?error, "api request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "服务器开小差了，请稍后再试")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut value = json!({ "error": self.message });
        if let Some(field) = self.field {
            value["field"] = json!(field);
        }
        (self.status, Json(value)).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::internal(error)
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::NotFound(message) => Self::new(StatusCode::NOT_FOUND, message),
            AppError::Forbidden(message) => Self::new(StatusCode::FORBIDDEN, message),
            AppError::BadRequest(message) => Self::new(StatusCode::BAD_REQUEST, message),
            AppError::Redirect(_) => Self::new(StatusCode::BAD_REQUEST, "请求没有完成"),
            AppError::Internal(error) => Self::internal(error),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
