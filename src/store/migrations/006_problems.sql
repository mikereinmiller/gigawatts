CREATE TABLE unrecognized_lines (
    path TEXT NOT NULL,
    kind TEXT NOT NULL,
    count INTEGER NOT NULL,
    first_seen_ms INTEGER NOT NULL,
    PRIMARY KEY (path, kind)
) WITHOUT ROWID;

CREATE INDEX requests_without_price ON requests (model_label, time_ms) WHERE cost_usd IS NULL;

CREATE INDEX session_models_by_model ON session_models (model_label);

CREATE INDEX session_models_with_price_gap ON session_models (model_label)
    WHERE price_gap_usd IS NOT NULL;

DELETE FROM store_settings WHERE key = 'price_table';
