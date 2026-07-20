-- Feature 28: federation robustness.

-- §7: last-applied owner `updated_at` for a received picture (stale-announcement guard).
ALTER TABLE pictures
    ADD COLUMN remote_updated_at TIMESTAMP NULL;

-- §8: the `federation_messages` machinery is 100% dead code. Drop the table, its indexes/FKs,
-- and its enum types.
DROP TABLE IF EXISTS federation_messages;
DROP TYPE IF EXISTS federation_message_type;
DROP TYPE IF EXISTS federation_direction;
DROP TYPE IF EXISTS federation_status;
