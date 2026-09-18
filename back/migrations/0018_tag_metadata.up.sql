-- Tag metadata (feature 34): one purely decorative side table keyed by (user_id, tag_path).
-- Nothing in the engine reads it — the pipeline, hierarchies, sharing and federation continue to
-- see only `tags`. WebDAV is the one deliberate exception (§2, §8): directory *naming* and the
-- *existence* of an empty directory.

CREATE TYPE tag_order AS ENUM ('manual', 'date_from', 'date_to', 'path', 'display_name');
CREATE TYPE tag_view_mode AS ENUM ('direct', 'subtag', 'all');
CREATE TYPE tag_subtag_placement AS ENUM ('top', 'in_sections');

CREATE TABLE tag_metadata
(
    user_id          uuid          NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    tag_path         ltree         NOT NULL,                            -- '' = the root view (§3.3)
    display_name     varchar(128),
    description      varchar(2000),
    cover_picture_id uuid REFERENCES pictures (id) ON DELETE SET NULL,
    color            varchar(16),                                       -- '#RRGGBB' (§3.2)
    date_from        timestamp,                                         -- NULL ⇒ derived (§4)
    date_to          timestamp,
    show_when_empty  boolean       NOT NULL DEFAULT false,
    sort_index       integer,                                           -- slot among siblings (§7)
    children_order   tag_order     NOT NULL DEFAULT 'manual',           -- how MY children sort
    view_mode        tag_view_mode NOT NULL DEFAULT 'subtag',           -- feature 35
    subtag_placement tag_subtag_placement,                              -- NULL ⇒ derived (§3.4)
    grouping         jsonb         NOT NULL DEFAULT '{}'::jsonb,        -- §3.1, feature 35 §4
    webdav_dir_name  varchar(255),                                      -- §8
    created_at       timestamp     NOT NULL DEFAULT (now() AT TIME ZONE 'utc'),
    updated_at       timestamp     NOT NULL DEFAULT (now() AT TIME ZONE 'utc'),
    PRIMARY KEY (user_id, tag_path)
);
-- The PK btree already narrows to one user; the GiST index only narrows the prefix for the
-- rename swap (§12). `idx_tag_metadata_cover` keeps a bulk purge from scanning the table once per
-- deleted picture to satisfy ON DELETE SET NULL.
CREATE INDEX idx_tag_metadata_path ON tag_metadata USING gist (tag_path);
CREATE INDEX idx_tag_metadata_cover ON tag_metadata USING btree (cover_picture_id)
    WHERE cover_picture_id IS NOT NULL;

-- Season grouping needs a hemisphere: the *viewer's* convention, applied to every photo regardless
-- of where it was taken (§3).
CREATE TYPE hemisphere AS ENUM ('north', 'south');
ALTER TABLE user_settings
    ADD COLUMN hemisphere hemisphere NOT NULL DEFAULT 'north';

-- The sender's decoration, carried on the share announcement and used to seed the recipient's
-- `SharedToMe.…` row on **first accept only** (§10.1). Parked here rather than seeded straight into
-- tag_metadata so a recipient who resets their metadata is never re-seeded.
ALTER TABLE incoming_shares
    ADD COLUMN sender_tag_meta jsonb;
