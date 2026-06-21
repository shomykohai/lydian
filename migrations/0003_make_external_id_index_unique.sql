DROP INDEX IF EXISTS idx_migrations_external_id;
DROP INDEX IF EXISTS idx_migrations_lookup;

CREATE UNIQUE INDEX IF NOT EXISTS idx_migrations_composite_unique
ON migrations (external_id, id_type, service);
