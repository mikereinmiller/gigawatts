CREATE TABLE directory_login_history (
    path TEXT NOT NULL,
    from_ms INTEGER NOT NULL,
    account_id INTEGER NOT NULL REFERENCES accounts (id),
    PRIMARY KEY (path, from_ms)
) WITHOUT ROWID;

INSERT INTO directory_login_history (path, from_ms, account_id)
SELECT path, 0, coalesce(previous_account_id, account_id) FROM directory_logins;

INSERT INTO directory_login_history (path, from_ms, account_id)
SELECT path, changed_at_ms, account_id FROM directory_logins
WHERE previous_account_id IS NOT NULL AND changed_at_ms IS NOT NULL;
