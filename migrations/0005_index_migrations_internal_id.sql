CREATE INDEX IF NOT EXISTS idx_migrations_internal_id
ON migrations (internal_id)
WHERE internal_id IS NOT NULL;
