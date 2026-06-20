ALTER TABLE migrations ADD COLUMN internal_id TEXT;

DROP INDEX IF EXISTS idx_migrations_lookup;

CREATE UNIQUE INDEX IF NOT EXISTS idx_migrations_lookup
ON migrations(external_id, id_type);
