-- Feature 20 — Calendar Segmentation + unified service config.
-- See doc/features/20_calendar_segmentation.md §10–§11.
--
-- Folds every service type's config into a single `config jsonb` column on `tagging_services`
-- and drops the three per-type child tables. shared_tag_mapping becomes one service per share;
-- old segmentation services are converted to plain `captured_at` range rule services.

ALTER TABLE tagging_services
    ADD COLUMN config jsonb NOT NULL DEFAULT '{}'::jsonb;

-- 1. rule services — inline the rule rows (array order = old `position`).
UPDATE tagging_services ts
SET config = jsonb_build_object('rules', COALESCE((SELECT jsonb_agg(jsonb_build_object(
                                                                            'id', r.id::text,
                                                                            'predicate', r.predicate,
                                                                            'assign_tag', r.assign_tag::text)
                                                                    ORDER BY r.position, r.id)
                                                   FROM rule_tagging_services r
                                                   WHERE r.service_id = ts.id), '[]'::jsonb))
WHERE ts.service_type = 'rule';

-- 2. segmentation services — old segments are plain `captured_at` ranges; convert each service
-- into a `rule` service (the band model's single root_tag can't represent arbitrary assign_tags).
UPDATE tagging_services ts
SET config       = jsonb_build_object('rules', COALESCE((SELECT jsonb_agg(jsonb_build_object(
                                                                                  'id', s.id::text,
                                                                                  'predicate', jsonb_build_object(
                                                                                          'field', 'captured_at',
                                                                                          'date_range', jsonb_build_object(
                                                                                                  'from',
                                                                                                  to_char(lower(s.date_range) AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS'),
                                                                                                  'to',
                                                                                                  to_char(upper(s.date_range) AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS'))),
                                                                                  'assign_tag', s.assign_tag::text)
                                                                          ORDER BY lower(s.date_range))
                                                         FROM segmentation_tagging_services s
                                                         WHERE s.service_id = ts.id), '[]'::jsonb)),
    service_type = 'rule'
WHERE ts.service_type = 'segmentation';

-- 3. shared_tag_mapping services — split each per-owner service into one service per mapping row.
-- `incoming_shares.local_mapping_service_id` points at the mapping-row id today; repoint it at the
-- new per-share service. Drop the FK first so the repoint can target tagging_services.
ALTER TABLE incoming_shares
    DROP CONSTRAINT fk_incoming_shares_mapping;

DO
$$
    DECLARE
        r      RECORD;
        new_id uuid;
    BEGIN
        FOR r IN
            SELECT stms.id               AS old_mapping_id,
                   stms.incoming_share_id,
                   stms.assign_tag::text AS assign_tag,
                   ts.owner_id,
                   ts.requires,
                   ts.excludes,
                   ts.enabled,
                   ts.position,
                   ts.name
            FROM shared_tag_mapping_services stms
                     JOIN tagging_services ts ON ts.id = stms.service_id
            LOOP
                INSERT INTO tagging_services (owner_id, service_type, requires, excludes, enabled,
                                              position, name, config)
                VALUES (r.owner_id, 'shared_tag_mapping', r.requires, r.excludes, r.enabled,
                        r.position, r.name,
                        jsonb_build_object('incoming_share_id', r.incoming_share_id::text,
                                           'assign_tags', jsonb_build_array(r.assign_tag)))
                RETURNING id INTO new_id;

                UPDATE incoming_shares
                SET local_mapping_service_id = new_id
                WHERE local_mapping_service_id = r.old_mapping_id;
            END LOOP;

        -- Delete the now-superseded original per-owner mapping services.
        DELETE
        FROM tagging_services
        WHERE service_type = 'shared_tag_mapping'
          AND id IN (SELECT DISTINCT service_id FROM shared_tag_mapping_services);
    END
$$;

ALTER TABLE incoming_shares
    ADD CONSTRAINT fk_incoming_shares_mapping
        FOREIGN KEY (local_mapping_service_id) REFERENCES tagging_services (id) ON DELETE SET NULL;

-- 4. Drop the per-type child tables — the evaluator reads `config` whole now.
DROP TABLE rule_tagging_services;
DROP TABLE segmentation_tagging_services;
DROP TABLE shared_tag_mapping_services;

-- Brokenness is derived from the share status now; index the share id the join keys on.
CREATE INDEX idx_tagging_services_mapping_share
    ON tagging_services ((config ->> 'incoming_share_id'))
    WHERE service_type = 'shared_tag_mapping';
