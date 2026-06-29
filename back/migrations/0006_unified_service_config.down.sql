-- Revert Feature 20 — unified service config. Structural revert only: the per-type rows are not
-- reconstructed from `config` (best-effort, matching feature 13's down migration).

DROP INDEX IF EXISTS idx_tagging_services_mapping_share;

-- Recreate the per-type child tables.
CREATE TABLE public.rule_tagging_services
(
    id         uuid    DEFAULT public.uuid_generate_v4() NOT NULL,
    service_id uuid                                      NOT NULL,
    predicate  jsonb                                     NOT NULL,
    assign_tag public.ltree                              NOT NULL,
    "position" integer DEFAULT 0                         NOT NULL
);

CREATE TABLE public.segmentation_tagging_services
(
    id                uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    service_id        uuid                                   NOT NULL,
    name              character varying(255)                 NOT NULL,
    date_range        tstzrange                              NOT NULL,
    assign_tag        public.ltree                           NOT NULL,
    parent_segment_id uuid
);

CREATE TABLE public.shared_tag_mapping_services
(
    id                uuid    DEFAULT public.uuid_generate_v4() NOT NULL,
    service_id        uuid                                      NOT NULL,
    incoming_share_id uuid                                      NOT NULL,
    assign_tag        public.ltree                              NOT NULL,
    is_broken         boolean DEFAULT false                     NOT NULL
);

ALTER TABLE ONLY public.rule_tagging_services
    ADD CONSTRAINT rule_tagging_services_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.segmentation_tagging_services
    ADD CONSTRAINT segmentation_tagging_services_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.shared_tag_mapping_services
    ADD CONSTRAINT shared_tag_mapping_services_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.rule_tagging_services
    ADD CONSTRAINT rule_tagging_services_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.tagging_services (id) ON DELETE CASCADE;
ALTER TABLE ONLY public.segmentation_tagging_services
    ADD CONSTRAINT segmentation_tagging_services_parent_segment_id_fkey FOREIGN KEY (parent_segment_id) REFERENCES public.segmentation_tagging_services (id) ON DELETE CASCADE;
ALTER TABLE ONLY public.segmentation_tagging_services
    ADD CONSTRAINT segmentation_tagging_services_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.tagging_services (id) ON DELETE CASCADE;
ALTER TABLE ONLY public.shared_tag_mapping_services
    ADD CONSTRAINT shared_tag_mapping_services_incoming_share_id_fkey FOREIGN KEY (incoming_share_id) REFERENCES public.incoming_shares (id) ON DELETE CASCADE;
ALTER TABLE ONLY public.shared_tag_mapping_services
    ADD CONSTRAINT shared_tag_mapping_services_service_id_fkey FOREIGN KEY (service_id) REFERENCES public.tagging_services (id) ON DELETE CASCADE;

CREATE INDEX idx_rts_position ON public.rule_tagging_services USING btree (service_id, "position");
CREATE INDEX idx_rts_service ON public.rule_tagging_services USING btree (service_id);
CREATE INDEX idx_stms_incoming_share ON public.shared_tag_mapping_services USING btree (incoming_share_id);
CREATE INDEX idx_stms_service ON public.shared_tag_mapping_services USING btree (service_id);
CREATE INDEX idx_sts_date_range ON public.segmentation_tagging_services USING gist (date_range);
CREATE INDEX idx_sts_parent ON public.segmentation_tagging_services USING btree (parent_segment_id);
CREATE INDEX idx_sts_service ON public.segmentation_tagging_services USING btree (service_id);

-- Repoint local_mapping_service_id back at the mapping-row table.
ALTER TABLE incoming_shares
    DROP CONSTRAINT fk_incoming_shares_mapping;
ALTER TABLE incoming_shares
    ADD CONSTRAINT fk_incoming_shares_mapping
        FOREIGN KEY (local_mapping_service_id) REFERENCES public.shared_tag_mapping_services (id) ON DELETE SET NULL;

ALTER TABLE tagging_services
    DROP COLUMN config;
