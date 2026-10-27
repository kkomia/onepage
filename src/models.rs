use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub is_super_admin: bool,
    pub is_active: bool,
    pub must_change_password: bool,
    pub last_login_at: Option<String>,
}

impl User {
    pub fn initials(&self) -> String {
        self.display_name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| self.username.chars().next().unwrap_or('?').to_uppercase().to_string())
    }
}

#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user: User,
    pub csrf: String,
    pub token_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Space {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "admin" => Role::Admin,
            _ => Role::Member,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpaceAccess {
    pub space: Space,
    pub role: Option<Role>,
    pub member: bool,
}

impl SpaceAccess {
    pub fn is_admin(&self, user: &User) -> bool {
        user.is_super_admin || self.role == Some(Role::Admin)
    }

    pub fn is_member(&self) -> bool {
        self.member
    }
}

#[derive(Debug, Clone)]
pub struct Category {
    pub id: i64,
    pub space_id: i64,
    pub owner_id: Option<i64>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct LinkRecord {
    pub id: i64,
    pub space_id: i64,
    pub owner_id: Option<i64>,
    pub category_id: Option<i64>,
    pub title: String,
    pub url: String,
    pub description: String,
    pub icon_hash: Option<String>,
    pub click_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct SpaceMembership {
    pub space: Space,
    pub role: Option<Role>,
    pub member: bool,
}
