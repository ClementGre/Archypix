ALTER TABLE incoming_shares
    DROP COLUMN sender_tag_meta;

ALTER TABLE user_settings
    DROP COLUMN hemisphere;
DROP TYPE hemisphere;

DROP TABLE tag_metadata;
DROP TYPE tag_subtag_placement;
DROP TYPE tag_view_mode;
DROP TYPE tag_order;
