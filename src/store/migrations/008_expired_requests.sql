CREATE TABLE expired_requests (
    session_id INTEGER NOT NULL REFERENCES sessions (id),
    model_label INTEGER NOT NULL REFERENCES labels (id),
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_5m_tokens INTEGER NOT NULL,
    cache_write_1h_tokens INTEGER NOT NULL,
    web_searches INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    PRIMARY KEY (session_id, model_label)
) WITHOUT ROWID;
ALTER TABLE sessions ADD COLUMN requests_skipped INTEGER NOT NULL DEFAULT 0;
