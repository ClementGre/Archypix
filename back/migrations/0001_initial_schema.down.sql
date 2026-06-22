-- ============================================================================
-- DROP ALL TABLES AND TYPES
-- ============================================================================

DROP TABLE IF EXISTS picture_versions CASCADE;
DROP TABLE IF EXISTS user_settings CASCADE;
DROP TABLE IF EXISTS federation_messages CASCADE;
DROP TABLE IF EXISTS jobs CASCADE;
DROP TABLE IF EXISTS hierarchies CASCADE;
DROP TABLE IF EXISTS segmentation_tagging_services CASCADE;
DROP TABLE IF EXISTS rule_tagging_services CASCADE;
DROP TABLE IF EXISTS shared_tag_mapping_services CASCADE;
DROP TABLE IF EXISTS tagging_services CASCADE;
DROP TABLE IF EXISTS share_announcements CASCADE;
DROP TABLE IF EXISTS incoming_shares CASCADE;
DROP TABLE IF EXISTS outgoing_shares CASCADE;
DROP TABLE IF EXISTS tags CASCADE;
DROP TABLE IF EXISTS pictures CASCADE;
DROP TABLE IF EXISTS refresh_tokens CASCADE;
DROP TABLE IF EXISTS user_credentials CASCADE;
DROP TABLE IF EXISTS users CASCADE;

DROP TYPE IF EXISTS share_status;
DROP TYPE IF EXISTS tag_source;
DROP TYPE IF EXISTS job_status;
DROP TYPE IF EXISTS job_type;
DROP TYPE IF EXISTS federation_message_type;
DROP TYPE IF EXISTS federation_direction;
DROP TYPE IF EXISTS federation_status;
DROP TYPE IF EXISTS safe_delete_mode;
DROP TYPE IF EXISTS service_type;
DROP TYPE IF EXISTS versioning_mode;
DROP TYPE IF EXISTS picture_exif_sync_status;
DROP TYPE IF EXISTS picture_deleted_reason;
