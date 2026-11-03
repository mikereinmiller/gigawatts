ALTER TABLE sessions ADD COLUMN account_id INTEGER REFERENCES accounts (id);

CREATE TABLE store_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
