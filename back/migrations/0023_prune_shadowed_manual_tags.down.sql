ALTER TABLE tagging_services
    DROP COLUMN IF EXISTS keep_manual_tags,
    DROP COLUMN IF EXISTS prune_shadowed_manual_tags;
