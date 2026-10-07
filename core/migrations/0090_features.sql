-- Batch A6: declared features, each tagged PII or not (see core/src/features.rs).
CREATE TABLE site_features (
    site_id TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    key     TEXT NOT NULL,
    pii     BOOLEAN NOT NULL,
    PRIMARY KEY (site_id, key)
);
