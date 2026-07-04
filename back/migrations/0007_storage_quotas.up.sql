-- Feature 22 — Storage quotas.
-- Per-user quota column + authoritative usage breakdown maintained by triggers.

-- ── Quota column (NULL = unlimited) ──────────────────────────────────────────
ALTER TABLE users
    ADD COLUMN storage_quota_bytes BIGINT;

-- ── Usage breakdown table (one row per user, four billed categories) ─────────
CREATE TABLE user_storage
(
    user_id                 uuid PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    originals_bytes         bigint                      NOT NULL DEFAULT 0, -- live owned originals
    originals_trashed_bytes bigint                      NOT NULL DEFAULT 0, -- trashed owned originals
    versions_bytes          bigint                      NOT NULL DEFAULT 0, -- versions of a live owned picture
    versions_trashed_bytes  bigint                      NOT NULL DEFAULT 0, -- versions of a trashed owned picture
    updated_at              timestamp without time zone NOT NULL DEFAULT (now() AT TIME ZONE 'utc')
);

-- ── Trigger functions applying signed deltas to the owner's user_storage row ──

-- Pictures: originals bucket delta (live/trashed by deleted_at, only when owned), plus the
-- picture's version bytes moving between the live/trashed version buckets when its trash state
-- flips. Attached AFTER INSERT/UPDATE and BEFORE DELETE: the BEFORE-DELETE timing lets it read the
-- picture's versions before the FK cascade removes them, and the child version trigger then skips
-- the cascade rows (parent already gone), so version bytes are subtracted exactly once.
CREATE FUNCTION user_storage_pictures() RETURNS trigger
    LANGUAGE plpgsql
AS
$$
DECLARE
    d_orig_live  bigint := 0;
    d_orig_trash bigint := 0;
    d_ver_live   bigint := 0;
    d_ver_trash  bigint := 0;
    vsum         bigint;
    uid          uuid;
BEGIN
    -- OLD contribution (UPDATE, DELETE).
    IF (TG_OP = 'UPDATE' OR TG_OP = 'DELETE') AND OLD.remote_picture_id IS NULL THEN
        IF OLD.deleted_at IS NULL THEN
            d_orig_live := d_orig_live - COALESCE(OLD.file_size, 0);
        ELSE
            d_orig_trash := d_orig_trash - COALESCE(OLD.file_size, 0);
        END IF;
    END IF;

    -- NEW contribution (INSERT, UPDATE).
    IF (TG_OP = 'INSERT' OR TG_OP = 'UPDATE') AND NEW.remote_picture_id IS NULL THEN
        IF NEW.deleted_at IS NULL THEN
            d_orig_live := d_orig_live + COALESCE(NEW.file_size, 0);
        ELSE
            d_orig_trash := d_orig_trash + COALESCE(NEW.file_size, 0);
        END IF;
    END IF;

    -- Trash-state flip: move this picture's version bytes between the version buckets.
    IF TG_OP = 'UPDATE' AND NEW.remote_picture_id IS NULL
        AND (OLD.deleted_at IS NULL) <> (NEW.deleted_at IS NULL) THEN
        SELECT COALESCE(SUM(file_size), 0) INTO vsum FROM picture_versions WHERE picture_id = NEW.id;
        IF NEW.deleted_at IS NULL THEN
            d_ver_live := d_ver_live + vsum;
            d_ver_trash := d_ver_trash - vsum;
        ELSE
            d_ver_live := d_ver_live - vsum;
            d_ver_trash := d_ver_trash + vsum;
        END IF;
    END IF;

    -- Delete: also drop this picture's version bytes before the cascade removes them.
    IF TG_OP = 'DELETE' AND OLD.remote_picture_id IS NULL THEN
        SELECT COALESCE(SUM(file_size), 0) INTO vsum FROM picture_versions WHERE picture_id = OLD.id;
        IF OLD.deleted_at IS NULL THEN
            d_ver_live := d_ver_live - vsum;
        ELSE
            d_ver_trash := d_ver_trash - vsum;
        END IF;
    END IF;

    IF TG_OP = 'DELETE' THEN uid := OLD.local_user_id; ELSE uid := NEW.local_user_id; END IF;

    IF d_orig_live <> 0 OR d_orig_trash <> 0 OR d_ver_live <> 0 OR d_ver_trash <> 0 THEN
        INSERT INTO user_storage (user_id, originals_bytes, originals_trashed_bytes,
                                  versions_bytes, versions_trashed_bytes)
        VALUES (uid, d_orig_live, d_orig_trash, d_ver_live, d_ver_trash)
        ON CONFLICT (user_id) DO UPDATE SET originals_bytes         = user_storage.originals_bytes + EXCLUDED.originals_bytes,
                                            originals_trashed_bytes = user_storage.originals_trashed_bytes + EXCLUDED.originals_trashed_bytes,
                                            versions_bytes          = user_storage.versions_bytes + EXCLUDED.versions_bytes,
                                            versions_trashed_bytes  = user_storage.versions_trashed_bytes + EXCLUDED.versions_trashed_bytes,
                                            updated_at              = (now() AT TIME ZONE 'utc');
    END IF;

    IF TG_OP = 'DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
END;
$$;

-- Picture versions: bill the version's bytes into the live/trashed version bucket of an owned
-- parent. Skips when the parent picture is gone (a cascade delete — the parent trigger already
-- accounted for it).
CREATE FUNCTION user_storage_versions() RETURNS trigger
    LANGUAGE plpgsql
AS
$$
DECLARE
    d_live    bigint := 0;
    d_trash   bigint := 0;
    uid       uuid;
    p_deleted boolean;
    p_owned   boolean;
BEGIN
    IF (TG_OP = 'UPDATE' OR TG_OP = 'DELETE') THEN
        SELECT (deleted_at IS NOT NULL), (remote_picture_id IS NULL), local_user_id
        INTO p_deleted, p_owned, uid
        FROM pictures
        WHERE id = OLD.picture_id;
        IF p_owned IS TRUE THEN
            IF p_deleted THEN
                d_trash := d_trash - COALESCE(OLD.file_size, 0);
            ELSE
                d_live := d_live - COALESCE(OLD.file_size, 0);
            END IF;
        END IF;
    END IF;

    IF (TG_OP = 'INSERT' OR TG_OP = 'UPDATE') THEN
        SELECT (deleted_at IS NOT NULL), (remote_picture_id IS NULL), local_user_id
        INTO p_deleted, p_owned, uid
        FROM pictures
        WHERE id = NEW.picture_id;
        IF p_owned IS TRUE THEN
            IF p_deleted THEN
                d_trash := d_trash + COALESCE(NEW.file_size, 0);
            ELSE
                d_live := d_live + COALESCE(NEW.file_size, 0);
            END IF;
        END IF;
    END IF;

    IF (d_live <> 0 OR d_trash <> 0) AND uid IS NOT NULL THEN
        INSERT INTO user_storage (user_id, versions_bytes, versions_trashed_bytes)
        VALUES (uid, d_live, d_trash)
        ON CONFLICT (user_id) DO UPDATE SET versions_bytes         = user_storage.versions_bytes + EXCLUDED.versions_bytes,
                                            versions_trashed_bytes = user_storage.versions_trashed_bytes + EXCLUDED.versions_trashed_bytes,
                                            updated_at             = (now() AT TIME ZONE 'utc');
    END IF;

    IF TG_OP = 'DELETE' THEN RETURN OLD; ELSE RETURN NEW; END IF;
END;
$$;

CREATE TRIGGER trg_user_storage_pictures_iu
    AFTER INSERT OR UPDATE OF file_size, deleted_at, remote_picture_id
    ON pictures
    FOR EACH ROW
EXECUTE FUNCTION user_storage_pictures();

CREATE TRIGGER trg_user_storage_pictures_del
    BEFORE DELETE
    ON pictures
    FOR EACH ROW
EXECUTE FUNCTION user_storage_pictures();

CREATE TRIGGER trg_user_storage_versions
    AFTER INSERT OR DELETE OR UPDATE OF file_size
    ON picture_versions
    FOR EACH ROW
EXECUTE FUNCTION user_storage_versions();

-- ── Backfill every existing user from the current picture/version rows ────────
INSERT INTO user_storage (user_id, originals_bytes, originals_trashed_bytes,
                          versions_bytes, versions_trashed_bytes)
SELECT u.id,
       COALESCE(o.live, 0),
       COALESCE(o.trash, 0),
       COALESCE(v.live, 0),
       COALESCE(v.trash, 0)
FROM users u
         LEFT JOIN (SELECT local_user_id                                        AS uid,
                           SUM(file_size) FILTER (WHERE deleted_at IS NULL)     AS live,
                           SUM(file_size) FILTER (WHERE deleted_at IS NOT NULL) AS trash
                    FROM pictures
                    WHERE remote_picture_id IS NULL
                    GROUP BY local_user_id) o ON o.uid = u.id
         LEFT JOIN (SELECT p.local_user_id                                           AS uid,
                           SUM(pv.file_size) FILTER (WHERE p.deleted_at IS NULL)     AS live,
                           SUM(pv.file_size) FILTER (WHERE p.deleted_at IS NOT NULL) AS trash
                    FROM picture_versions pv
                             JOIN pictures p ON p.id = pv.picture_id
                    WHERE p.remote_picture_id IS NULL
                    GROUP BY p.local_user_id) v ON v.uid = u.id
ON CONFLICT (user_id) DO UPDATE SET originals_bytes         = EXCLUDED.originals_bytes,
                                    originals_trashed_bytes = EXCLUDED.originals_trashed_bytes,
                                    versions_bytes          = EXCLUDED.versions_bytes,
                                    versions_trashed_bytes  = EXCLUDED.versions_trashed_bytes;
