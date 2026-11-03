CREATE TABLE directories (
    cwd_label INTEGER PRIMARY KEY REFERENCES labels (id),
    repo_name TEXT NOT NULL,
    repo_path TEXT,
    remote_url TEXT
);

ALTER TABLE requests ADD COLUMN head_branch_label INTEGER REFERENCES labels (id);

CREATE INDEX requests_by_session ON requests (session_id);

ALTER TABLE sessions ADD COLUMN cost_status TEXT;

ALTER TABLE sessions ADD COLUMN cost_forgotten INTEGER NOT NULL DEFAULT 0;

ALTER TABLE sessions ADD COLUMN cost_set_aside INTEGER NOT NULL DEFAULT 0;

ALTER TABLE session_models ADD COLUMN price_gap_usd REAL;

CREATE TABLE not_in_transcript (
    session_id INTEGER NOT NULL REFERENCES sessions (id),
    model_label INTEGER NOT NULL REFERENCES labels (id),
    day TEXT,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_5m_tokens INTEGER NOT NULL,
    cache_write_1h_tokens INTEGER NOT NULL,
    web_searches INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    PRIMARY KEY (session_id, model_label)
) WITHOUT ROWID;

CREATE INDEX not_in_transcript_by_day ON not_in_transcript (day);

ALTER TABLE daily_totals ADD COLUMN branch_label INTEGER REFERENCES labels (id);

DELETE FROM daily_totals;

INSERT INTO daily_totals (
    day, account_id, cwd_label, branch_label, skill_label, agent_label, model_label, requests,
    input_tokens, output_tokens, thinking_tokens, cache_read_tokens,
    cache_write_5m_tokens, cache_write_1h_tokens, web_searches, web_fetches, cost_usd
)
SELECT day, account_id, cwd_label, git_branch_label, skill_label, agent_label, model_label,
       count(*), sum(input_tokens), sum(output_tokens), sum(thinking_tokens),
       sum(cache_read_tokens), sum(cache_write_5m_tokens), sum(cache_write_1h_tokens),
       sum(web_searches), sum(web_fetches), sum(cost_usd)
FROM requests
WHERE day IS NOT NULL
GROUP BY day, account_id, cwd_label, git_branch_label, skill_label, agent_label, model_label;

DELETE FROM file_positions;

DELETE FROM store_settings WHERE key = 'price_table';
