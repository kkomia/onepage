PRAGMA foreign_keys = ON;

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL
);

CREATE TABLE users (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    username             TEXT NOT NULL COLLATE NOCASE UNIQUE,
    display_name         TEXT NOT NULL,
    password_hash        TEXT NOT NULL,
    is_super_admin       INTEGER NOT NULL DEFAULT 0,
    is_active            INTEGER NOT NULL DEFAULT 1,
    must_change_password INTEGER NOT NULL DEFAULT 0,
    created_at           TEXT NOT NULL,
    last_login_at        TEXT
);

CREATE TABLE spaces (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    slug        TEXT NOT NULL COLLATE NOCASE UNIQUE,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_by  INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE memberships (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    space_id   INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role       TEXT NOT NULL CHECK (role IN ('admin', 'member')),
    created_at TEXT NOT NULL,
    UNIQUE (space_id, user_id)
);

CREATE INDEX idx_memberships_user ON memberships(user_id);
CREATE INDEX idx_memberships_space ON memberships(space_id);

CREATE TABLE categories (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    space_id   INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    owner_id   INTEGER REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_categories_scope ON categories(space_id, owner_id, sort_order, id);

CREATE TABLE icons (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    hash         TEXT NOT NULL UNIQUE,
    content_type TEXT NOT NULL,
    data         BLOB NOT NULL,
    source_url   TEXT NOT NULL,
    fetched_at   TEXT NOT NULL
);

CREATE TABLE links (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    space_id      INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    owner_id      INTEGER REFERENCES users(id) ON DELETE CASCADE,
    category_id   INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    title         TEXT NOT NULL,
    url           TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    icon_id       INTEGER REFERENCES icons(id) ON DELETE SET NULL,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    click_count   INTEGER NOT NULL DEFAULT 0,
    last_clicked_at TEXT,
    created_by    INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE INDEX idx_links_scope ON links(space_id, owner_id, category_id, sort_order, id);
CREATE INDEX idx_links_public ON links(space_id) WHERE owner_id IS NULL;

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    csrf       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL,
    ip         TEXT NOT NULL DEFAULT '',
    user_agent TEXT NOT NULL DEFAULT ''
);

CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_expiry ON sessions(expires_at);
