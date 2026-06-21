CREATE INDEX idx_migrations_pending_files
ON migrations(output_file_path)
WHERE service != 'local' AND output_file_path IS NOT NULL;
