CREATE TABLE link_clicks (
    link_id         INTEGER NOT NULL REFERENCES links(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    click_count     INTEGER NOT NULL DEFAULT 0,
    last_clicked_at TEXT,
    PRIMARY KEY (link_id, user_id)
);

CREATE INDEX idx_link_clicks_user ON link_clicks(user_id, link_id);
