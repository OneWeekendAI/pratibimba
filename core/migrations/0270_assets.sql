-- S11: the asset store. One row per (site, SHA-256 of the bytes); the bytes live in the store
-- (a local directory or an S3 bucket) under `<site>/<sha256>`. `unreferenced_since` is set by the
-- worker's garbage collection when no live variant of the site names the asset, cleared when one
-- does again; a row unreferenced for the grace period is deleted with its bytes.
CREATE TABLE assets (
    site_id             TEXT NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    sha256              TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    mime                TEXT NOT NULL CHECK (mime IN ('image/png', 'image/jpeg', 'image/webp', 'image/avif', 'image/gif', 'image/svg+xml')),
    bytes               INT NOT NULL CHECK (bytes > 0),
    width               INT CHECK (width > 0),
    height              INT CHECK (height > 0),
    name                TEXT CHECK (char_length(name) <= 128),
    uploaded_by         TEXT NOT NULL,
    mcp_call_id         BIGINT REFERENCES mcp_calls(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- An upload starts unreferenced: its grace period runs from the upload.
    unreferenced_since  TIMESTAMPTZ DEFAULT now(),
    PRIMARY KEY (site_id, sha256)
);
CREATE INDEX assets_unreferenced ON assets (unreferenced_since) WHERE unreferenced_since IS NOT NULL;
