CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    account_uuid TEXT NOT NULL,
    organization_uuid TEXT NOT NULL,
    UNIQUE (account_uuid, organization_uuid)
);

CREATE TABLE labels (
    id INTEGER PRIMARY KEY,
    text TEXT NOT NULL UNIQUE
);

CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    uuid TEXT NOT NULL UNIQUE,
    title TEXT,
    total_cost_usd REAL,
    start_time_ms INTEGER,
    api_duration_ms INTEGER,
    api_duration_without_retries_ms INTEGER,
    tool_duration_ms INTEGER,
    duration_ms INTEGER,
    lines_added INTEGER,
    lines_removed INTEGER,
    has_unknown_model_cost INTEGER
);

CREATE TABLE session_models (
    session_id INTEGER NOT NULL REFERENCES sessions (id),
    model_label INTEGER NOT NULL REFERENCES labels (id),
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    thinking_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_5m_tokens INTEGER NOT NULL,
    cache_write_1h_tokens INTEGER NOT NULL,
    has_cache_write_split INTEGER NOT NULL,
    web_searches INTEGER NOT NULL,
    web_fetches INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    PRIMARY KEY (session_id, model_label)
) WITHOUT ROWID;

CREATE TABLE requests (
    id INTEGER PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE,
    session_id INTEGER NOT NULL REFERENCES sessions (id),
    account_id INTEGER REFERENCES accounts (id),
    time_ms INTEGER,
    day TEXT,
    model_label INTEGER NOT NULL REFERENCES labels (id),
    requested_model_label INTEGER REFERENCES labels (id),
    advisor_model_label INTEGER REFERENCES labels (id),
    effort_label INTEGER REFERENCES labels (id),
    per_turn_effort_label INTEGER REFERENCES labels (id),
    speed_label INTEGER REFERENCES labels (id),
    inference_geo_label INTEGER REFERENCES labels (id),
    service_tier_label INTEGER REFERENCES labels (id),
    stop_reason_label INTEGER REFERENCES labels (id),
    entrypoint_label INTEGER REFERENCES labels (id),
    version_label INTEGER REFERENCES labels (id),
    user_type_label INTEGER REFERENCES labels (id),
    cwd_label INTEGER REFERENCES labels (id),
    git_branch_label INTEGER REFERENCES labels (id),
    skill_label INTEGER REFERENCES labels (id),
    agent_label INTEGER REFERENCES labels (id),
    agent_id TEXT,
    is_sidechain INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    thinking_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_5m_tokens INTEGER NOT NULL,
    cache_write_1h_tokens INTEGER NOT NULL,
    web_searches INTEGER NOT NULL,
    web_fetches INTEGER NOT NULL,
    thinking_duration_ms INTEGER,
    cost_usd REAL
);

CREATE INDEX requests_by_day ON requests (day);

CREATE TABLE request_tools (
    request_row_id INTEGER NOT NULL REFERENCES requests (id) ON DELETE CASCADE,
    tool_label INTEGER NOT NULL REFERENCES labels (id),
    PRIMARY KEY (request_row_id, tool_label)
) WITHOUT ROWID;

CREATE TABLE daily_totals (
    day TEXT NOT NULL,
    account_id INTEGER REFERENCES accounts (id),
    cwd_label INTEGER REFERENCES labels (id),
    skill_label INTEGER REFERENCES labels (id),
    agent_label INTEGER REFERENCES labels (id),
    model_label INTEGER NOT NULL REFERENCES labels (id),
    requests INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    thinking_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_5m_tokens INTEGER NOT NULL,
    cache_write_1h_tokens INTEGER NOT NULL,
    web_searches INTEGER NOT NULL,
    web_fetches INTEGER NOT NULL,
    cost_usd REAL
);

CREATE INDEX daily_totals_by_day ON daily_totals (day);

CREATE TABLE file_positions (
    path TEXT PRIMARY KEY,
    device INTEGER NOT NULL,
    inode INTEGER NOT NULL,
    first_line_hash INTEGER,
    offset INTEGER NOT NULL
) WITHOUT ROWID;
