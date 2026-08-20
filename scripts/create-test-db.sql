-- Runs once, on first start of the postgres container.
-- The integration tests truncate tables, so they get their own database.
CREATE DATABASE pilight_test OWNER pilight;
