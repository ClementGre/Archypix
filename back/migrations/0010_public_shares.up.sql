-- Public shares (feature 27): link-gated *pull* shares served entirely by the owner backend.
-- Coverage is computed live at request time (like the hierarchy resolver) — no IncomingShare, no
-- per-picture tokens, no pipeline involvement. Only "convert to a derived share" re-enters the
-- share pipeline (via the ordinary outgoing_shares below).

CREATE TYPE public_share_status AS ENUM ('active', 'revoked');

CREATE TABLE public_shares
(
    id                   uuid PRIMARY KEY             DEFAULT uuid_generate_v4(),
    owner_id             uuid                NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    tag_path             ltree               NOT NULL,                  -- what's covered
    name                 varchar(64)         NOT NULL,
    message              text,
    token                text                NOT NULL UNIQUE,           -- 256-bit base64url secret in the URL
    password_hash        text,                                          -- optional access gate (argon2)
    expires_at           timestamp,                                     -- optional (UTC-naive, like the rest of the schema)
    allow_originals      boolean             NOT NULL DEFAULT false,    -- download + copy + convert-to-share
    allow_upload         boolean             NOT NULL DEFAULT false,    -- anonymous contribution
    allow_share_back     boolean             NOT NULL DEFAULT false,    -- authed ShareBack (forced on if allow_upload)
    conv_allow_exif_edit boolean             NOT NULL DEFAULT false,    -- inherited by the derived share on Subscribe
    conv_future          boolean             NOT NULL DEFAULT true,     -- inherited by the derived share on Subscribe
    status               public_share_status NOT NULL DEFAULT 'active', -- active | revoked
    created_at           timestamp           NOT NULL DEFAULT (now() AT TIME ZONE 'utc'),
    revoked_at           timestamp
);
CREATE INDEX idx_public_shares_owner ON public_shares (owner_id);
CREATE INDEX idx_public_shares_tag ON public_shares USING gist (tag_path);
-- token lookup is by the UNIQUE constraint.

-- Provenance for a derived share minted on Subscribe + the key for the revoke-time cascade prompt.
ALTER TABLE outgoing_shares
    ADD COLUMN derived_from_public_share_id uuid REFERENCES public_shares (id) ON DELETE SET NULL;
