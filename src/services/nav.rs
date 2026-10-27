use std::collections::HashMap;

use anyhow::Result;
use serde_json::{json, Value};

use crate::models::{SessionUser, SpaceAccess};
use crate::services::{links, settings, spaces};
use crate::state::AppState;

pub async fn build_bootstrap(
    state: &AppState,
    session: &SessionUser,
    access: &SpaceAccess,
    flash: Option<Value>,
) -> Result<Value> {
    let db = &state.db;
    let public_categories = links::categories_for(db, access.space.id, None).await?;
    let personal_categories = if access.is_member() {
        links::categories_for(db, access.space.id, Some(session.user.id)).await?
    } else {
        Vec::new()
    };
    let searchable = links::searchable_links(
        db,
        access.space.id,
        if access.is_member() {
            Some(session.user.id)
        } else {
            None
        },
    )
    .await?;

    let mut counts: HashMap<i64, i64> = HashMap::new();
    for link in &searchable {
        if let Some(category_id) = link.category_id {
            *counts.entry(category_id).or_insert(0) += 1;
        }
    }

    let mut categories = Vec::with_capacity(public_categories.len() + personal_categories.len());
    for category in &public_categories {
        categories.push(json!({
            "id": category.id,
            "name": category.name,
            "scope": "public",
            "count": counts.get(&category.id).copied().unwrap_or(0),
        }));
    }
    for category in &personal_categories {
        categories.push(json!({
            "id": category.id,
            "name": category.name,
            "scope": "mine",
            "count": counts.get(&category.id).copied().unwrap_or(0),
        }));
    }

    let links_json = searchable
        .into_iter()
        .map(|link| {
            json!({
                "id": link.id,
                "title": link.title,
                "url": link.url,
                "description": link.description,
                "categoryId": link.category_id,
                "scope": if link.public { "public" } else { "mine" },
                "iconUrl": link.icon_hash.map(|hash| format!("/icons/{hash}")),
                "clickCount": link.click_count,
                "createdAt": link.created_at,
                "py": link.py,
                "pyi": link.pyi,
            })
        })
        .collect::<Vec<_>>();

    let switch_spaces = spaces::all_spaces_for_switcher(db, &session.user).await?;
    let spaces_json = switch_spaces
        .into_iter()
        .map(|membership| {
            json!({
                "slug": membership.space.slug,
                "name": membership.space.name,
                "description": membership.space.description,
                "role": membership.role.map(|role| role.as_str()),
                "member": membership.member,
                "current": membership.space.id == access.space.id,
            })
        })
        .collect::<Vec<_>>();

    let site_name = settings::site_name(db).await?;
    let role = if session.user.is_super_admin && !access.is_member() {
        "super"
    } else {
        access.role.map(|role| role.as_str()).unwrap_or("viewer")
    };

    Ok(json!({
        "siteName": site_name,
        "user": {
            "id": session.user.id,
            "name": session.user.display_name,
            "username": session.user.username,
            "initials": session.user.initials(),
            "isSuperAdmin": session.user.is_super_admin,
        },
        "space": {
            "slug": access.space.slug,
            "name": access.space.name,
            "description": access.space.description,
        },
        "role": role,
        "isAdmin": access.is_admin(&session.user),
        "isMember": access.is_member(),
        "spaces": spaces_json,
        "categories": categories,
        "links": links_json,
        "csrf": session.csrf,
        "apiBase": format!("/api/s/{}", access.space.slug),
        "urls": {
            "public": format!("/s/{}", access.space.slug),
            "admin": format!("/s/{}/admin/links", access.space.slug),
            "globalAdmin": "/admin/spaces",
            "password": "/account/password",
            "logout": "/logout",
            "root": "/",
        },
        "iconFetch": state.config.icon_fetch,
        "version": crate::templates::ASSET_VERSION,
        "flash": flash.unwrap_or(Value::Null),
    }))
}
