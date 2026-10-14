-- Per-service opt-in to prune manual tags the service also produces, down to the gate-derived
-- anchor, plus a user floor below which a manual tag is never reduced (feature 39 §4).
ALTER TABLE tagging_services
    ADD COLUMN prune_shadowed_manual_tags boolean NOT NULL DEFAULT false,
    ADD COLUMN keep_manual_tags           ltree[] NOT NULL DEFAULT '{}';
