CREATE TABLE IF NOT EXISTS migrations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL,
    id_type TEXT NOT NULL,
    output_file_path TEXT,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    service TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_migrations_external_id
ON migrations (external_id);

CREATE INDEX IF NOT EXISTS idx_migrations_lookup
ON migrations(external_id, id_type, service);
