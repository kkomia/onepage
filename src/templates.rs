use std::sync::Arc;

use anyhow::Context;
use axum::response::Html;
use minijinja::Environment;
use serde_json::{json, Value};

use crate::error::AppResult;

pub const ASSET_VERSION: &str = env!("ONEPAGE_ASSET_VERSION");

pub fn environment() -> Arc<Environment<'static>> {
    let mut env = Environment::new();

    macro_rules! add {
        ($name:literal) => {
            env.add_template($name, include_str!(concat!("../templates/", $name)))
                .expect("embedded template parses");
        };
    }

    add!("auth_base.html");
    add!("login.html");
    add!("setup.html");
    add!("password.html");
    add!("no_space.html");
    add!("nav.html");
    add!("admin_base.html");
    add!("admin/links.html");
    add!("admin/categories.html");
    add!("admin/members.html");
    add!("admin/settings.html");
    add!("admin/spaces.html");
    add!("admin/users.html");
    add!("admin/site.html");

    Arc::new(env)
}

pub fn render(env: &Environment<'static>, name: &str, context: Value) -> AppResult<Html<String>> {
    let template = env
        .get_template(name)
        .with_context(|| format!("模板不存在: {name}"))?;
    let body = template
        .render(minijinja::value::Serde(&context))
        .with_context(|| format!("模板渲染失败: {name}"))?;
    Ok(Html(body))
}

pub fn base_context(site_name: &str, page: &str) -> Value {
    json!({
        "site_name": site_name,
        "page": page,
        "asset_version": ASSET_VERSION,
    })
}
