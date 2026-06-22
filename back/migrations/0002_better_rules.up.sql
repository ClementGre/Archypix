-- Feature 13 — Better Rules. See doc/features/13_better_rules.md.
--
-- This migration bundles three related changes:
--   1. Rule predicates move from free-text function-call strings to a structured JSONB
--      predicate tree (arbitrary AND/OR/NOT composition, typed field conditions).
--   2. rule_tagging_services rows gain an explicit, user-controllable display order.
--   3. tagging_services gain a user-facing name so the pipeline list is legible.

-- 1. Convert legacy text predicates to JSONB.
--
-- Existing text predicates follow the four known legacy forms; convert them in place. Anything
-- unrecognised becomes a never-match predicate (`{"or": []}`) so the column stays valid JSON.
CREATE FUNCTION pg_temp.convert_legacy_predicate(p text) RETURNS jsonb AS
$$
DECLARE
    m text[];
BEGIN
    m := regexp_match(p, '^\s*gps_within_bbox\(\s*([0-9.eE+-]+)\s*,\s*([0-9.eE+-]+)\s*,\s*([0-9.eE+-]+)\s*,\s*([0-9.eE+-]+)\s*\)\s*$');
    IF m IS NOT NULL THEN
        RETURN jsonb_build_object('gps_bbox', jsonb_build_object(
                'lat_min', m[1]::numeric, 'lat_max', m[2]::numeric,
                'lon_min', m[3]::numeric, 'lon_max', m[4]::numeric));
    END IF;

    m := regexp_match(p, '^\s*capture_year\(\s*(\d+)\s*\)\s*$');
    IF m IS NOT NULL THEN
        RETURN jsonb_build_object('field', 'captured_at', 'year', m[1]::int);
    END IF;

    m := regexp_match(p, '^\s*capture_month\(\s*(\d+)\s*\)\s*$');
    IF m IS NOT NULL THEN
        RETURN jsonb_build_object('field', 'captured_at', 'month', m[1]::int);
    END IF;

    m := regexp_match(p, '^\s*filename_contains\(\s*"(.*)"\s*\)\s*$');
    IF m IS NOT NULL THEN
        RETURN jsonb_build_object('field', 'filename', 'contains', m[1]);
    END IF;

    -- Unknown legacy predicate: keep it inert (never matches) but structurally valid.
    RETURN jsonb_build_object('or', jsonb_build_array());
END;
$$ LANGUAGE plpgsql;

ALTER TABLE rule_tagging_services
    ALTER COLUMN predicate TYPE jsonb USING pg_temp.convert_legacy_predicate(predicate);

-- 2. Add an explicit display order to rules (rules are evaluated independently, so position is
-- presentation only). Existing rows keep a stable order seeded from their creation order per service.
ALTER TABLE rule_tagging_services
    ADD COLUMN position INT NOT NULL DEFAULT 0;

WITH ordered AS (SELECT id, row_number() OVER (PARTITION BY service_id ORDER BY id) - 1 AS pos
                 FROM rule_tagging_services)
UPDATE rule_tagging_services r
SET position = ordered.pos
FROM ordered
WHERE r.id = ordered.id;

CREATE INDEX idx_rts_position ON rule_tagging_services (service_id, position);

-- 3. Add a user-facing name for tagging services.
ALTER TABLE tagging_services
    ADD COLUMN name VARCHAR(255) NOT NULL DEFAULT '';
