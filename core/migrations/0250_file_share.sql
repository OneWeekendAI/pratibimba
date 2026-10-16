-- The traffic share the site file stated for a variant at its last apply (NULL: it stated none).
-- A re-apply changes `traffic_share` only when the file's value differs from this, so an owner's
-- edit (approvals action 'share') survives re-applying an unchanged file.
ALTER TABLE variants ADD COLUMN file_share REAL;
