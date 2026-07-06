-- Feature 23: fleet admin control plane — heartbeat state, capacity, runtime config, invites,
-- operator token.

-- Per-backend state pushed by the heartbeat (§3.2) + resolver-side capacity policy (§7.3).
ALTER TABLE backends
    ADD COLUMN delegation_token        TEXT,
    ADD COLUMN delegation_expires_at   TIMESTAMPTZ,
    ADD COLUMN user_count              BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN picture_count           BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN storage_bytes           BIGINT  NOT NULL DEFAULT 0,
    ADD COLUMN last_heartbeat_at       TIMESTAMPTZ,
    ADD COLUMN healthy                 BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN reachable               BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN accepting_registrations BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN max_users               BIGINT,
    ADD COLUMN version                 TEXT,
    -- Round-robin cursor: selection picks the eligible backend least-recently chosen.
    ADD COLUMN last_selected_at        TIMESTAMPTZ;

-- Resolver's own runtime config (feature 23 §4.6): default < env(locks) < DB override.
CREATE TABLE resolver_settings
(
    key        TEXT PRIMARY KEY,
    value      JSONB       NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Invite store (feature 23 §6.2). Resolver invites may carry an instance_pin (§7.2).
CREATE TABLE invites
(
    code         VARCHAR(255) PRIMARY KEY,
    max_uses     BIGINT,
    uses         BIGINT       NOT NULL DEFAULT 0,
    expires_at   TIMESTAMPTZ,
    created_by   VARCHAR(255) NOT NULL,
    instance_pin VARCHAR(255),
    created_at   TIMESTAMPTZ  NOT NULL DEFAULT now()
);

-- Single-row operator credential (feature 23 §5.1): hashed operator token + auto-rotating refresh.
CREATE TABLE resolver_admin
(
    id                 INTEGER PRIMARY KEY  DEFAULT 1 CHECK (id = 1),
    token_hash         TEXT        NOT NULL,
    refresh_token_hash TEXT,
    refresh_expires_at TIMESTAMPTZ,
    rotated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
