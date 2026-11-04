CREATE TABLE limit_fetches (
    account_id INTEGER PRIMARY KEY REFERENCES accounts (id),
    fetched_at_ms INTEGER NOT NULL,
    source TEXT NOT NULL,
    limits TEXT NOT NULL
);

CREATE TABLE limit_readings (
    account_id INTEGER NOT NULL REFERENCES accounts (id),
    kind TEXT NOT NULL,
    scope TEXT NOT NULL,
    fetched_at_ms INTEGER NOT NULL,
    percent REAL NOT NULL,
    resets_at TEXT,
    PRIMARY KEY (account_id, kind, scope, fetched_at_ms)
) WITHOUT ROWID;

CREATE TABLE usage_advice (
    account_id INTEGER NOT NULL REFERENCES accounts (id),
    day TEXT NOT NULL,
    fetched_at_ms INTEGER NOT NULL,
    behaviors TEXT NOT NULL,
    PRIMARY KEY (account_id, day)
) WITHOUT ROWID;
