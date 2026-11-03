ALTER TABLE accounts ADD COLUMN email TEXT;

CREATE INDEX requests_by_account ON requests (account_id);

CREATE TABLE directory_logins (
    path TEXT PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts (id),
    previous_account_id INTEGER REFERENCES accounts (id),
    changed_at_ms INTEGER
) WITHOUT ROWID;
