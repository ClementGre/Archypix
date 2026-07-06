-- Feature 23: runtime config + registration rules (standalone backend).

-- Runtime config DB overrides (feature 23 §4.3). Layered under env: default < env(locks) < DB.
CREATE TABLE app_settings
(
    key        TEXT PRIMARY KEY,
    value      JSONB       NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Who invited each user (feature 23 §6.3); nullable username (global domain implicit).
ALTER TABLE users
    ADD COLUMN invited_by VARCHAR(255);

-- Standalone invite store (feature 23 §6.2). No instance_pin (resolver-only).
CREATE TABLE invites
(
    code       VARCHAR(255) PRIMARY KEY,
    max_uses   BIGINT,
    uses       BIGINT       NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ,
    created_by VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ  NOT NULL DEFAULT now()
);
