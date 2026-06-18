-- Enable required extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "ltree";

-- ============================================================================
-- ENUM TYPES
-- ============================================================================
CREATE TYPE share_status AS ENUM ('pending', 'pending_first_announcement', 'active', 'errored', 'revoked', 'tombstoned');
CREATE TYPE tag_source AS ENUM ('manual', 'rule', 'segment', 'share_mapping', 'incoming_share');
CREATE TYPE job_status AS ENUM ('pending', 'processing', 'completed', 'failed');
CREATE TYPE job_type AS ENUM ('gen_thumbnail', 'ml_style', 'ml_people', 'ml_group_location', 'edit_picture');
CREATE TYPE federation_message_type AS ENUM ('share_announcement', 'share_revocation', 'picture_update');
CREATE TYPE federation_direction AS ENUM ('inbound', 'outbound');
CREATE TYPE federation_status AS ENUM ('pending', 'sent', 'delivered', 'failed');
CREATE TYPE safe_delete_mode AS ENUM ('singleBranch', 'fullDelete');
CREATE TYPE service_type AS ENUM ('shared_tag_mapping', 'rule', 'segmentation');
CREATE TYPE picture_exif_sync_status AS ENUM ('synced', 'pending', 'unsupported');


-- ============================================================================
-- USERS
-- ============================================================================
CREATE TABLE users
(
    id           UUID PRIMARY KEY      DEFAULT uuid_generate_v4(),
    username     VARCHAR(255) NOT NULL,
    email        VARCHAR(255) NOT NULL,
    display_name VARCHAR(255) NOT NULL,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    created_at   TIMESTAMP    NOT NULL DEFAULT (now() at time zone 'utc'),
    updated_at   TIMESTAMP    NOT NULL DEFAULT (now() at time zone 'utc'),

    -- Composite unique constraint for federation identity
    CONSTRAINT uq_user_username UNIQUE (username),
    CONSTRAINT uq_user_email UNIQUE (email)
);

CREATE INDEX idx_users_username ON users (username);
CREATE INDEX idx_users_email ON users (email);

-- ============================================================================
-- USER CREDENTIALS
-- ============================================================================
CREATE TABLE user_credentials
(
    user_id       UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    password_hash TEXT      NOT NULL,
    created_at    TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc'),
    updated_at    TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc')
);

CREATE INDEX idx_user_credentials_user ON user_credentials (user_id);

-- ============================================================================
-- REFRESH TOKENS
-- ============================================================================
CREATE TABLE refresh_tokens
(
    id         UUID PRIMARY KEY   DEFAULT uuid_generate_v4(),
    user_id    UUID      NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT      NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    revoked_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc'),
    updated_at TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc'),

    CONSTRAINT uq_refresh_token_hash UNIQUE (token_hash)
);

CREATE INDEX idx_refresh_tokens_user ON refresh_tokens (user_id);
CREATE INDEX idx_refresh_tokens_expires ON refresh_tokens (expires_at);

-- ============================================================================
-- PICTURES
-- ============================================================================
CREATE TABLE pictures
(
    id                    UUID PRIMARY KEY       DEFAULT uuid_generate_v4(),

    -- Local user holding this picture row (owner for own pictures, recipient for received pictures)
    local_user_id     UUID          NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    -- For received pictures only: the original owner's picture id (their `id`). NULL for owned pictures.
    -- Used as deduplication key to avoid inserting the same foreign picture twice (e.g. via two share paths).
    remote_picture_id VARCHAR(255),

    -- Cross-instance support: original owner identity (for received pictures)
    owner_username        VARCHAR(255),           -- NULL for owned pictures
    owner_instance_domain VARCHAR(255),           -- NULL for owned pictures

    -- Metadata
    filename              VARCHAR(1024),
    mime_type             VARCHAR(100),
    file_size             BIGINT,
    width                 INTEGER,
    height                INTEGER,

    -- EXIF and other metadata (flexible JSONB)
    exif_data         JSONB         NOT NULL DEFAULT '{}',

    -- ML/processing results
    metadata          JSONB         NOT NULL DEFAULT '{}',

    -- Soft deletion (local only - received pictures never physically deleted)
    deleted_at            TIMESTAMP,

    -- Timestamps
    captured_at           TIMESTAMP,              -- From EXIF
    ingested_at           TIMESTAMP     NOT NULL DEFAULT (now() at time zone 'utc'),
    updated_at              TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc'),

    -- Worker-populated fields (NULL until processing completes)
    -- BlurHash for progressive loading in the UI
    blurhash                TEXT,
    -- GPS coordinates extracted from EXIF (DOUBLE PRECISION for direct f64 mapping)
    gps_lat                 DOUBLE PRECISION,
    gps_lng                 DOUBLE PRECISION,
    gps_alt                 INTEGER,
    -- EXIF orientation tag (1=normal, 3=180°, 6=90°CW, 8=90°CCW, etc.)
    orientation             SMALLINT,
    -- Set to NOW() when a worker first generates thumbnails for this picture.
    -- NULL means thumbnails have never been generated.
    thumbnails_generated_at TIMESTAMP,
    -- SHA-256 hex digest of the stored file; computed by workers.
    -- Serves as the WebDAV ETag (future WebDAV implementation).
    file_hash            TEXT,

    -- Tagging pipeline: NULL means the picture has never been processed (dirty by default).
    -- Set to NOW() after a successful pipeline run; reset to NULL on manual tag changes.
    last_pipeline_run_at TIMESTAMP,

    -- Convergence of the S3 original's embedded EXIF versus this row (the source of truth).
    -- 'pending' while an edit_picture job rewrites the file; 'unsupported' when the format
    -- cannot embed EXIF (DB-only edit, no job); 'synced' otherwise.
    exif_sync_status     picture_exif_sync_status NOT NULL DEFAULT 'synced'
);

CREATE INDEX idx_pictures_local_user ON pictures (local_user_id);
-- Deduplication for received pictures: one row per (recipient, remote picture id)
CREATE UNIQUE INDEX uq_received_picture ON pictures (local_user_id, remote_picture_id)
    WHERE remote_picture_id IS NOT NULL;
CREATE INDEX idx_pictures_deleted ON pictures (deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX idx_pictures_captured ON pictures (captured_at);
CREATE INDEX idx_pictures_exif ON pictures USING GIN (exif_data);
CREATE INDEX idx_pictures_metadata ON pictures USING GIN (metadata);
CREATE INDEX idx_pictures_remote_owner ON pictures (owner_username, owner_instance_domain)
    WHERE owner_username IS NOT NULL;

-- ============================================================================
-- TAGS
-- ============================================================================
CREATE TABLE tags
(
    id          UUID PRIMARY KEY    DEFAULT uuid_generate_v4(),
    picture_id  UUID       NOT NULL REFERENCES pictures (id) ON DELETE CASCADE,

    -- Tag path using ltree for hierarchy (e.g., 'Photos.Travel.Alps')
    -- Stored without leading slash, case-sensitive; only explicit tags stored, ancestors derived on read
    tag_path    LTREE      NOT NULL,

    -- Source of the tag assignment.
    -- source_id is polymorphic (no FK): the tagging_services.id for pipeline sources
    -- (rule/segment/share_mapping), or the incoming_shares.id for incoming_share tags;
    -- NULL for manual tags. Lifecycle (live re-derivation by the pipeline, removal on
    -- service disable, promotion to manual on service delete) is enforced in the app layer.
    source      tag_source NOT NULL DEFAULT 'manual',
    source_id   UUID,

    -- Per-picture presign token (only populated for source = 'incoming_share' rows).
    -- Stores the token the sender generated in `share_announcements` and forwarded in the
    -- `AnnouncedPicture`. Used to authorise presign calls to the sender on the local user's
    -- clients' behalf, and forwarded downstream in transitive announcements.
    picture_token UUID,

    -- Timestamps
    assigned_at TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc')
);

-- Tags are keyed per-source: the same path may be asserted independently by several
-- sources on one picture (e.g. a manual tag and a rule that also matches it).
--   * manual tags    : at most one row per (picture, path); source_id is always NULL.
--   * non-manual tags: at most one row per (picture, path, source, producing source_id).
-- These partial unique indexes also serve as the ON CONFLICT arbiters for tag writers.
CREATE UNIQUE INDEX uq_picture_tag_manual ON tags (picture_id, tag_path) WHERE source = 'manual';
CREATE UNIQUE INDEX uq_picture_tag_source ON tags (picture_id, tag_path, source, source_id) WHERE source <> 'manual';

-- GiST index for efficient ltree operations (@>, <@, ~, @)
CREATE INDEX idx_tags_path ON tags USING GIST (tag_path);
CREATE INDEX idx_tags_picture ON tags (picture_id);
CREATE INDEX idx_tags_source ON tags (source, source_id);
-- Index over presign tokens stored on incoming_share tag rows (transitive token selection).
-- Not unique: on a single-instance deployment the same forwarded token can appear on two
-- recipients' tag rows (e.g. Bob's and Carol's) for the same upstream picture.
CREATE INDEX idx_tags_picture_token ON tags (picture_token) WHERE picture_token IS NOT NULL;

-- ============================================================================
-- OUTGOING SHARES
-- ============================================================================
CREATE TABLE outgoing_shares
(
    id                 UUID PRIMARY KEY      DEFAULT uuid_generate_v4(),
    owner_id           UUID         NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    -- What is shared
    tag_path           LTREE        NOT NULL,              -- Tag being shared

    name    VARCHAR(64) NOT NULL,
    message TEXT,

    -- Who receives it
    recipient_username VARCHAR(255) NOT NULL,
    recipient_instance VARCHAR(255) NOT NULL,

    -- Share configuration
    allow_share_back   BOOLEAN      NOT NULL DEFAULT TRUE,
    future             BOOLEAN      NOT NULL DEFAULT TRUE, -- Auto-announce new pictures

    -- ShareBack provenance: the original OutgoingShare this share was created in response to
    -- (i.e. the recipient's incoming share's outgoing_share_id). NULL for normal shares. Kept for
    -- end-user display only; no FK (the referenced row is local but may be deleted independently).
    shareback_of UUID,

    -- Status: starts as pending until the recipient accepts, then active. A delivery failure on
    -- an active future share demotes it to 'errored'; the pipeline retries it with a full reconcile.
    status share_status NOT NULL DEFAULT 'pending',

    -- Announcement retry/backoff (set on a failed delivery, cleared on success).
    last_error_at TIMESTAMP,
    next_retry_at TIMESTAMP,

    -- Timestamps
    created_at         TIMESTAMP    NOT NULL DEFAULT (now() at time zone 'utc'),
    revoked_at TIMESTAMP
);

CREATE INDEX idx_outgoing_shares_owner ON outgoing_shares (owner_id);
CREATE INDEX idx_outgoing_shares_recipient ON outgoing_shares (recipient_username, recipient_instance);
CREATE INDEX idx_outgoing_shares_tag ON outgoing_shares USING GIST (tag_path);
CREATE INDEX idx_outgoing_shares_status ON outgoing_shares (status);
-- Only one live (non-terminal) share per (owner, tag, recipient) at a time.
-- Revoked/tombstoned rows are excluded so re-sharing after revocation is allowed.
CREATE UNIQUE INDEX uq_outgoing_share ON outgoing_shares (owner_id, tag_path, recipient_username, recipient_instance)
    WHERE status NOT IN ('revoked'::share_status, 'tombstoned'::share_status);

-- ============================================================================
-- SHARE ANNOUNCEMENTS (per-picture presign tokens — sender-side tracking table)
-- ============================================================================
-- One row per (outgoing_share, picture) that has been announced to the recipient.
-- Each row carries a unique `picture_token` that the recipient (and any transitive
-- recipient) presents to this backend's presign endpoint to fetch the picture.
-- The pipeline announcement step diffs current share coverage against this table to
-- decide which pictures to announce / unannounce. Revoking a share deletes its rows,
-- immediately invalidating every token it held.
CREATE TABLE share_announcements
(
    outgoing_share_id UUID NOT NULL REFERENCES outgoing_shares (id) ON DELETE CASCADE,
    picture_id        UUID NOT NULL, -- sender's local picture.id
    picture_token     UUID NOT NULL DEFAULT gen_random_uuid(),
    -- The pictures.updated_at value captured at the last successful (re-)announce of this picture.
    -- Gates metadata re-announce: a picture is re-announced when pictures.updated_at moves past it.
    announced_updated_at TIMESTAMP,
    PRIMARY KEY (outgoing_share_id, picture_id)
);

CREATE INDEX idx_share_announcements_picture ON share_announcements (picture_id);
-- Token → picture lookup for the presign endpoint. Not unique: a relayer copies the upstream
-- owner's token into its own tracking row for a transitively-shared picture, so the same token
-- value can appear on both the owner's row and the relayer's row (on a single-instance
-- deployment). The presign lookup disambiguates by resolving only the *owned* picture.
CREATE INDEX idx_share_announcements_token ON share_announcements (picture_token);

-- ============================================================================
-- INCOMING SHARES
-- ============================================================================
CREATE TABLE incoming_shares
(
    id                       UUID PRIMARY KEY      DEFAULT uuid_generate_v4(),
    recipient_id             UUID         NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    -- Who sent it
    sender_username          VARCHAR(255) NOT NULL,
    sender_instance          VARCHAR(255) NOT NULL,

    name    VARCHAR(64) NOT NULL,
    message TEXT,

    -- Reference to sender's OutgoingShare
    outgoing_share_id        UUID         NOT NULL, -- No FK (cross-instance)

    -- Local mapping service (optional)
    local_mapping_service_id UUID,                  -- FK added after tagging_services table

    -- Status: starts as pending until the recipient explicitly accepts.
    status share_status NOT NULL DEFAULT 'pending',

    -- Propagated from the sender's ShareAnnouncement: whether the recipient may share
    -- these pictures back to the sender with auto-accept. Drives the "Share back" UI.
    allow_share_back BOOLEAN NOT NULL DEFAULT FALSE,

    -- Propagated from the sender's OutgoingShare: whether new pictures under the tag are
    -- auto-announced. Display only (the sender owns the actual auto-announce behaviour).
    future                        BOOLEAN NOT NULL DEFAULT FALSE,

    -- The local `/SharedToMe/<sender>/…` tag these pictures land under, derived from the sender's
    -- shared tag_path. Set on share creation and refreshed on each picture announcement (so a
    -- sender-side tag rename / re-target is reflected). Advisory/display only — the authoritative
    -- per-picture tag is the announcement-driven `incoming_share` tag row.
    shared_tag_path               LTREE,

    -- When the sender last announced pictures for this share (NULL until the first announcement).
    last_announcement_received_at TIMESTAMP,

    -- ShareBack provenance: the recipient's own OutgoingShare this incoming share is a share-back
    -- of. NULL for normal shares. Kept for end-user display only; no FK.
    shareback_of                  UUID,

    -- Timestamps
    created_at               TIMESTAMP    NOT NULL DEFAULT (now() at time zone 'utc'),
    revoked_at               TIMESTAMP,

    -- Composite unique: one incoming share per sender per outgoing share
    CONSTRAINT uq_incoming_share UNIQUE (recipient_id, sender_username, sender_instance, outgoing_share_id)
);

CREATE INDEX idx_incoming_shares_recipient ON incoming_shares (recipient_id);
CREATE INDEX idx_incoming_shares_sender ON incoming_shares (sender_username, sender_instance);
CREATE INDEX idx_incoming_shares_status ON incoming_shares (status);

-- ============================================================================
-- TAGGING SERVICES (Base table)
-- ============================================================================
CREATE TABLE tagging_services
(
    id           UUID PRIMARY KEY      DEFAULT uuid_generate_v4(),
    owner_id     UUID         NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    -- Service type discriminator (determines pipeline order and trigger labels)
    service_type service_type NOT NULL,

    -- Gate conditions
    requires LTREE[] NOT NULL DEFAULT '{}', -- Tags required for service to fire
    excludes LTREE[] NOT NULL DEFAULT '{}', -- Tags that prevent service from firing

    -- Status
    enabled      BOOLEAN      NOT NULL DEFAULT TRUE,

    -- Execution order within Rule/Segmentation services (SharedTagMapping always runs first).
    position INT NOT NULL DEFAULT 0,

    -- Pipeline tracking: bumped on any configuration change; pictures with
    -- last_pipeline_run_at < last_invalidated_at are considered dirty.
    last_invalidated_at TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc'),
    -- Last pipeline error for this service (cleared on next successful run).
    last_error_at       TIMESTAMP,
    last_error_msg      TEXT,

    -- Timestamps
    created_at   TIMESTAMP    NOT NULL DEFAULT (now() at time zone 'utc'),
    updated_at   TIMESTAMP    NOT NULL DEFAULT (now() at time zone 'utc')
);

CREATE INDEX idx_tagging_services_owner ON tagging_services (owner_id);
CREATE INDEX idx_tagging_services_type ON tagging_services (service_type);
CREATE INDEX idx_tagging_services_enabled ON tagging_services (enabled);

-- ============================================================================
-- SHARED TAG MAPPING SERVICE
-- ============================================================================
CREATE TABLE shared_tag_mapping_services
(
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    service_id        UUID    NOT NULL REFERENCES tagging_services (id) ON DELETE CASCADE,

    -- Which incoming share to map
    incoming_share_id UUID    NOT NULL REFERENCES incoming_shares (id) ON DELETE CASCADE,

    -- Tag to assign
    assign_tag        LTREE   NOT NULL,

    -- Status (flagged if incoming share is revoked)
    is_broken         BOOLEAN NOT NULL DEFAULT FALSE,

    -- Unique mapping per service per incoming share
    CONSTRAINT uq_stms_mapping UNIQUE (service_id, incoming_share_id)
);

CREATE INDEX idx_stms_service ON shared_tag_mapping_services (service_id);
CREATE INDEX idx_stms_incoming_share ON shared_tag_mapping_services (incoming_share_id);

-- Add FK from incoming_shares to shared_tag_mapping_services
ALTER TABLE incoming_shares
    ADD CONSTRAINT fk_incoming_shares_mapping
        FOREIGN KEY (local_mapping_service_id)
            REFERENCES shared_tag_mapping_services (id)
            ON DELETE SET NULL;

-- ============================================================================
-- RULE TAGGING SERVICE
-- ============================================================================
CREATE TABLE rule_tagging_services
(
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    service_id UUID  NOT NULL REFERENCES tagging_services (id) ON DELETE CASCADE,

    -- Predicate expression (e.g., "exif.gps within bbox(45.8, 6.8, 46.1, 7.1)")
    predicate  TEXT  NOT NULL,

    -- Tag to assign
    assign_tag LTREE NOT NULL
);

CREATE INDEX idx_rts_service ON rule_tagging_services (service_id);

-- ============================================================================
-- SEGMENTATION TAGGING SERVICE
-- ============================================================================
CREATE TABLE segmentation_tagging_services
(
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    service_id        UUID         NOT NULL REFERENCES tagging_services (id) ON DELETE CASCADE,

    -- Segment definition
    name              VARCHAR(255) NOT NULL,

    -- Date range (stored as tsrange for efficient overlap queries)
    date_range        TSTZRANGE    NOT NULL,

    -- Tag to assign
    assign_tag        LTREE        NOT NULL,

    -- Parent segment for subsegments
    parent_segment_id UUID REFERENCES segmentation_tagging_services (id) ON DELETE CASCADE
);

CREATE INDEX idx_sts_service ON segmentation_tagging_services (service_id);
CREATE INDEX idx_sts_date_range ON segmentation_tagging_services USING GIST (date_range);
CREATE INDEX idx_sts_parent ON segmentation_tagging_services (parent_segment_id);

-- ============================================================================
-- HIERARCHIES (WebDAV filesystem mappings)
-- ============================================================================
CREATE TABLE hierarchies
(
    id         UUID PRIMARY KEY      DEFAULT uuid_generate_v4(),
    owner_id   UUID         NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    -- Hierarchy name
    name       VARCHAR(255) NOT NULL,

    -- Configuration as JSONB: an ordered node tree mapping the tag graph to a directory tree.
    -- See doc/features/05_hierarchies.md §4 for the full schema.
    config     JSONB        NOT NULL DEFAULT '{
      "version": 1,
      "safeDeleteMode": "singleBranch",
      "naming": "original",
      "writeBack": true,
      "nodes": []
    }',

    -- Status
    enabled    BOOLEAN      NOT NULL DEFAULT TRUE,

    -- WebDAV: one per-hierarchy access token (the HTTP Basic password for the mount),
    -- AES-256-GCM encrypted at rest (nonce ‖ ciphertext ‖ tag) with an HKDF sub-key of
    -- JWT_SECRET. NULL ⇒ no token issued yet. See doc/features/06_webdav.md §3.
    webdav_token_enc    BYTEA,
    -- WebDAV read strategy: true ⇒ GET responds 302 to a presigned URL; false ⇒ the
    -- backend proxies the bytes (for clients that don't follow redirects). §6.
    webdav_use_redirect BOOLEAN NOT NULL DEFAULT TRUE,

    -- Timestamps
    created_at TIMESTAMP    NOT NULL DEFAULT (now() at time zone 'utc'),
    updated_at TIMESTAMP    NOT NULL DEFAULT (now() at time zone 'utc'),

    -- Unique name per owner
    CONSTRAINT uq_hierarchy_name UNIQUE (owner_id, name)
);

CREATE INDEX idx_hierarchies_owner ON hierarchies (owner_id);

-- Config JSONB structure (node tree — see doc/features/05_hierarchies.md §4):
-- {
--   "version": 1,
--   "safeDeleteMode": "singleBranch" | "fullDelete",
--   "naming": "original" | "date" | "id",
--   "writeBack": true,                         -- master read-only switch
--   "nodes": [                                 -- ordered tree of directory nodes
--     {
--       "id": "n_photos", "kind": "mirror", "name": "Photos",
--       "tagRoot": "Photos", "keepDir": false,
--       "collapsed": ["Photos.Travel.Alps.Hiking"],
--       "exclude": ["Photos.Outdoor"]
--     },
--     {
--       "id": "n_fav", "kind": "query", "name": "Favorites",
--       "match": "all", "include": ["Starred"], "exclude": [], "matchUntagged": false,
--       "writeBack": { "onAdd": [{"op": "assign", "path": "Starred"}],
--                      "onRemove": [{"op": "remove", "path": "Starred"}] },
--       "children": []
--     },
--     { "id": "n_albums", "kind": "static", "name": "Albums", "children": [] }
--   ]
-- }

-- ============================================================================
-- JOBS (Async processing queue)
-- ============================================================================
CREATE TABLE jobs
(
    id              UUID PRIMARY KEY    DEFAULT uuid_generate_v4(),
    owner_id        UUID       NOT NULL REFERENCES users (id) ON DELETE CASCADE,

    -- Job type
    job_type        job_type   NOT NULL,

    -- Job status
    status          job_status NOT NULL DEFAULT 'pending',

    -- Configuration (job-specific params, may include picture IDs)
    config          JSONB      NOT NULL DEFAULT '{}',

    -- Result (populated when completed)
    result          JSONB               DEFAULT '{}',

    -- Error handling
    error_message   TEXT,
    retry_count     INTEGER    NOT NULL DEFAULT 0,
    max_retries     INTEGER    NOT NULL DEFAULT 3,

    -- Idempotency
    idempotency_key VARCHAR(255) UNIQUE,

    -- Primary picture for single-picture jobs (NULL for batch jobs)
    picture_id UUID REFERENCES pictures (id) ON DELETE CASCADE,
    -- Worker instance ID while status = 'processing'
    claimed_by TEXT,
    -- One-time UUID issued at claim time; the worker must echo it back in
    -- complete/fail so stale workers cannot corrupt a re-claimed job.
    claim_token UUID,

    -- Timestamps
    created_at      TIMESTAMP  NOT NULL DEFAULT (now() at time zone 'utc'),
    started_at      TIMESTAMP,
    completed_at    TIMESTAMP,

    -- Ensure idempotency
    CONSTRAINT uq_job_idempotency UNIQUE (owner_id, idempotency_key)
);

CREATE INDEX idx_jobs_owner ON jobs (owner_id);
CREATE INDEX idx_jobs_status ON jobs (status);
CREATE INDEX idx_jobs_type ON jobs (job_type);
CREATE INDEX idx_jobs_created ON jobs (created_at);
-- Partial index for fast job claiming: only pending jobs, ordered by created_at
CREATE INDEX idx_jobs_pending_claim ON jobs (job_type, created_at) WHERE status = 'pending';
-- Index for looking up jobs by picture
CREATE INDEX idx_jobs_picture ON jobs (picture_id) WHERE picture_id IS NOT NULL;
-- At most one in-flight edit_picture reconcile per picture (the concurrency rule for EXIF edits):
-- a second concurrent edit folds into the pending job or waits for the in-flight one to complete.
CREATE UNIQUE INDEX uq_edit_picture_inflight ON jobs (picture_id)
    WHERE job_type = 'edit_picture' AND status IN ('pending', 'processing');
-- Index for GPS bbox queries (used by rule-based tagging)
CREATE INDEX idx_pictures_gps ON pictures (gps_lat, gps_lng) WHERE gps_lat IS NOT NULL;
-- Index for efficient dirty-picture queries (pipeline loop)
CREATE INDEX idx_pictures_pipeline ON pictures (local_user_id, last_pipeline_run_at);
-- Partial index for the stuck-`pending` EXIF resync sweep (pictures awaiting file reconcile).
CREATE INDEX idx_pictures_exif_pending ON pictures (id) WHERE exif_sync_status = 'pending';

-- Config JSONB structure examples:
-- gen_thumbnail: {"picture_ids": ["uuid1", "uuid2"], "sizes": ["thumb", "medium"]}
-- ml_people: {"picture_ids": ["uuid1"], "snapshot_version": "v1.2.3"}
-- ml_group_location: {"picture_ids": ["uuid1", "uuid2", "uuid3"]}

-- ============================================================================
-- FEDERATION MESSAGES (Outbound/inbound federation log)
-- ============================================================================
CREATE TABLE federation_messages
(
    id                 UUID PRIMARY KEY                 DEFAULT uuid_generate_v4(),

    -- Message type
    message_type       federation_message_type NOT NULL,

    -- Direction
    direction          federation_direction    NOT NULL,

    -- Source/destination
    sender_username    VARCHAR(255),
    sender_instance    VARCHAR(255),
    recipient_username VARCHAR(255),
    recipient_instance VARCHAR(255),

    -- Related entities (optional, for correlation)
    outgoing_share_id  UUID                    REFERENCES outgoing_shares (id) ON DELETE SET NULL,
    incoming_share_id  UUID                    REFERENCES incoming_shares (id) ON DELETE SET NULL,

    -- Payload (may include picture IDs and other data)
    payload            JSONB                   NOT NULL DEFAULT '{}',

    -- Status
    status             federation_status       NOT NULL DEFAULT 'pending',

    -- Timestamps
    created_at         TIMESTAMP               NOT NULL DEFAULT (now() at time zone 'utc'),
    sent_at            TIMESTAMP,
    delivered_at       TIMESTAMP,

    -- Idempotency key to prevent duplicate processing of the same federation message
    idempotency_key TEXT UNIQUE,

    -- Error handling
    error_message      TEXT,
    retry_count        INTEGER                 NOT NULL DEFAULT 0
);

CREATE INDEX idx_federation_messages_type ON federation_messages (message_type);
CREATE INDEX idx_federation_messages_direction ON federation_messages (direction);
CREATE INDEX idx_federation_messages_status ON federation_messages (status);
CREATE INDEX idx_federation_messages_sender ON federation_messages (sender_username, sender_instance);
CREATE INDEX idx_federation_messages_recipient ON federation_messages (recipient_username, recipient_instance);

-- Payload JSONB structure examples:
-- share_announcement: {"picture_ids": ["uuid1", "uuid2"], "tag_path": "Photos.Travel.Alps"}
-- share_revocation: {"reason": "user_request"}
-- picture_update: {"picture_ids": ["uuid1"], "update_type": "metadata"}

-- ============================================================================
-- HELPER FUNCTIONS
-- ============================================================================

-- Function to check if a picture has a tag (including virtual ancestors)
-- Note: get_tag_ancestors is implemented in Rust using ltree operators
CREATE OR REPLACE FUNCTION picture_has_tag(picture_uuid UUID, target_tag LTREE)
    RETURNS BOOLEAN AS
$$
BEGIN
    RETURN EXISTS (SELECT 1
                   FROM tags
                   WHERE picture_id = picture_uuid
                     AND (
                       tag_path = target_tag -- exact match
                           OR tag_path <@ target_tag -- stored tag is a descendant; target is a virtual ancestor
                       ));
END;
$$ LANGUAGE plpgsql STABLE;

-- Function to get all pictures under a tag (including descendants)
CREATE OR REPLACE FUNCTION get_pictures_under_tag(tag_prefix LTREE)
    RETURNS TABLE
            (
                picture_id UUID
            )
AS
$$
BEGIN
    RETURN QUERY
        SELECT DISTINCT t.picture_id
        FROM tags t
        WHERE t.tag_path <@ tag_prefix -- tag_path is descendant of or equal to tag_prefix
          AND NOT EXISTS (SELECT 1
                          FROM pictures p
                          WHERE p.id = t.picture_id
                            AND p.deleted_at IS NOT NULL);
END;
$$ LANGUAGE plpgsql STABLE;

-- ============================================================================
-- TRIGGERS
-- ============================================================================

-- Auto-update updated_at timestamp
CREATE OR REPLACE FUNCTION update_updated_at_column()
    RETURNS TRIGGER AS
$$
BEGIN
    NEW.updated_at = (now() at time zone 'utc');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_users_updated_at
    BEFORE UPDATE
    ON users
    FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_user_credentials_updated_at
    BEFORE UPDATE
    ON user_credentials
    FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_refresh_tokens_updated_at
    BEFORE UPDATE
    ON refresh_tokens
    FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_pictures_updated_at
    BEFORE UPDATE
    ON pictures
    FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_tagging_services_updated_at
    BEFORE UPDATE
    ON tagging_services
    FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER update_hierarchies_updated_at
    BEFORE UPDATE
    ON hierarchies
    FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();

-- ============================================================================
-- COMMENTS (Documentation)
-- ============================================================================

COMMENT ON TABLE users IS 'User accounts with federation identity (@username:instance.com)';
COMMENT ON TABLE user_credentials IS 'Password hashes for local authentication (Argon2)';
COMMENT ON TABLE refresh_tokens IS 'Hashed refresh tokens with expiry and revocation';
COMMENT ON TABLE pictures IS 'Picture metadata; id is the federation identifier for owned pictures, remote_picture_id stores the foreign id for received pictures';
COMMENT ON TABLE tags IS 'Tag assignments using ltree for hierarchical paths; only explicit tags stored, ancestors derived on read';
COMMENT ON TABLE outgoing_shares IS 'Shares created by users to share tags with other users';
COMMENT ON TABLE incoming_shares IS 'Shares received from other users';
COMMENT ON TABLE tagging_services IS 'Base table for tagging service pipeline; service_type determines order and triggers';
COMMENT ON TABLE shared_tag_mapping_services IS 'Maps incoming shares to local tags';
COMMENT ON TABLE rule_tagging_services IS 'Assigns tags based on EXIF/metadata predicates';
COMMENT ON TABLE segmentation_tagging_services IS 'Assigns tags based on date ranges with subsegment support';
COMMENT ON TABLE hierarchies IS 'Tag-graph → directory-tree mappings; config JSONB stores an ordered node tree (mirror/query/static), safeDeleteMode, naming, writeBack — see doc/features/05_hierarchies.md §4. webdav_token_enc/webdav_use_redirect drive the WebDAV mount — see doc/features/06_webdav.md.';
COMMENT ON TABLE jobs IS 'Async processing queue; config JSONB holds job-specific params (may include picture IDs)';
COMMENT ON TABLE federation_messages IS 'Federation message log; payload JSONB holds message data (may include picture IDs)';

COMMENT ON FUNCTION picture_has_tag IS 'Checks if a picture has a tag including virtual ancestors';
COMMENT ON FUNCTION get_pictures_under_tag IS 'Returns all non-deleted pictures under a tag prefix';

-- ============================================================================
-- VERSIONING MODE
-- ============================================================================
CREATE TYPE versioning_mode AS ENUM ('none', 'original_copy', 'full_versioning');

-- ============================================================================
-- USER SETTINGS (one row per user, created lazily with defaults)
-- ============================================================================
CREATE TABLE user_settings
(
    user_id         UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    versioning_mode versioning_mode NOT NULL DEFAULT 'none',
    created_at      TIMESTAMP       NOT NULL DEFAULT (now() at time zone 'utc'),
    updated_at      TIMESTAMP       NOT NULL DEFAULT (now() at time zone 'utc')
);

CREATE TRIGGER update_user_settings_updated_at
    BEFORE UPDATE
    ON user_settings
    FOR EACH ROW
EXECUTE FUNCTION update_updated_at_column();

COMMENT ON TABLE user_settings IS 'Per-user preferences; created on first access with defaults';

-- ============================================================================
-- PICTURE VERSIONS
-- ============================================================================
CREATE TABLE picture_versions
(
    -- id is the version UUID used as the S3 key suffix: {user_id}/{picture_id}/{id}
    -- No DEFAULT — callers must pass the same UUID they used for the S3 copy.
    id UUID PRIMARY KEY,
    picture_id     UUID      NOT NULL REFERENCES pictures (id) ON DELETE CASCADE,
    version_number INT       NOT NULL,
    file_size      BIGINT,
    mime_type      VARCHAR(100),
    created_at     TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc'),

    CONSTRAINT uq_picture_version UNIQUE (picture_id, version_number)
);

CREATE INDEX idx_picture_versions_picture ON picture_versions (picture_id);

COMMENT ON TABLE picture_versions IS 'Versioned snapshots of picture files in archypix-versions bucket; S3 key = {user_id}/{picture_id}/{id}';
