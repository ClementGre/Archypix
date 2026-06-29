
CREATE SCHEMA public;

CREATE TYPE public.federation_direction AS ENUM (
    'inbound',
    'outbound'
);

CREATE TYPE public.federation_message_type AS ENUM (
    'share_announcement',
    'share_revocation',
    'picture_update'
);

CREATE TYPE public.federation_status AS ENUM (
    'pending',
    'sent',
    'delivered',
    'failed'
);

CREATE TYPE public.job_status AS ENUM (
    'pending',
    'processing',
    'completed',
    'failed'
);

CREATE TYPE public.job_type AS ENUM (
    'gen_thumbnail',
    'ml_style',
    'ml_people',
    'ml_group_location',
    'edit_picture'
);

CREATE TYPE public.picture_deleted_reason AS ENUM (
    'manual',
    'boomerang',
    'content_dedupe'
);

CREATE TYPE public.picture_exif_sync_status AS ENUM (
    'synced',
    'pending',
    'unsupported',
    'pending_job_creation'
);

CREATE TYPE public.safe_delete_mode AS ENUM (
    'singleBranch',
    'fullDelete'
);

CREATE TYPE public.service_type AS ENUM (
    'shared_tag_mapping',
    'rule',
    'segmentation'
);

CREATE TYPE public.share_status AS ENUM (
    'pending',
    'pending_first_announcement',
    'active',
    'errored',
    'revoked',
    'tombstoned'
);

CREATE TYPE public.tag_source AS ENUM (
    'manual',
    'rule',
    'segment',
    'share_mapping',
    'incoming_share'
);

CREATE TYPE public.versioning_mode AS ENUM (
    'none',
    'original_copy',
    'full_versioning'
);

CREATE FUNCTION public.get_pictures_under_tag(tag_prefix public.ltree)
    RETURNS TABLE
            (
                picture_id uuid
            )
    LANGUAGE plpgsql STABLE
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
$$;

CREATE FUNCTION public.picture_has_tag(picture_uuid uuid, target_tag public.ltree) RETURNS boolean
    LANGUAGE plpgsql STABLE
AS
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
$$;

CREATE FUNCTION public.update_updated_at_column() RETURNS trigger
    LANGUAGE plpgsql
AS
$$
BEGIN
    NEW.updated_at = (now() at time zone 'utc');
    RETURN NEW;
END;
$$;

CREATE TABLE public.federation_messages
(
    id              uuid                        DEFAULT public.uuid_generate_v4()        NOT NULL,
    message_type public.federation_message_type NOT NULL,
    direction public.federation_direction NOT NULL,
    sender_username character varying(255),
    sender_instance character varying(255),
    recipient_username character varying(255),
    recipient_instance character varying(255),
    outgoing_share_id uuid,
    incoming_share_id uuid,
    payload         jsonb                       DEFAULT '{}'::jsonb                      NOT NULL,
    status public.federation_status DEFAULT 'pending'::public.federation_status NOT NULL,
    created_at      timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    sent_at         timestamp without time zone,
    delivered_at    timestamp without time zone,
    idempotency_key text,
    error_message   text,
    retry_count     integer                     DEFAULT 0                                NOT NULL
);

CREATE TABLE public.hierarchies
(
    id                  uuid                        DEFAULT public.uuid_generate_v4()        NOT NULL,
    owner_id            uuid                                                                 NOT NULL,
    name                character varying(255)                                               NOT NULL,
    config              jsonb                       DEFAULT '{
      "nodes": [],
      "naming": "original",
      "version": 1,
      "writeBack": true,
      "safeDeleteMode": "singleBranch"
    }'::jsonb                                                                                NOT NULL,
    enabled             boolean                     DEFAULT true                             NOT NULL,
    webdav_token_enc    bytea,
    webdav_use_redirect boolean                     DEFAULT true                             NOT NULL,
    created_at          timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    updated_at          timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL
);

CREATE TABLE public.incoming_shares
(
    id                uuid                        DEFAULT public.uuid_generate_v4()        NOT NULL,
    recipient_id      uuid                                                                 NOT NULL,
    sender_username   character varying(255)                                               NOT NULL,
    sender_instance   character varying(255)                                               NOT NULL,
    name              character varying(64)                                                NOT NULL,
    message           text,
    outgoing_share_id uuid                                                                 NOT NULL,
    local_mapping_service_id uuid,
    status public.share_status DEFAULT 'pending'::public.share_status NOT NULL,
    allow_share_back  boolean                     DEFAULT false                            NOT NULL,
    future            boolean                     DEFAULT false                            NOT NULL,
    allow_exif_edit   boolean                     DEFAULT false                            NOT NULL,
    shared_tag_path public.ltree,
    last_announcement_received_at timestamp without time zone,
    shareback_of      uuid,
    created_at        timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    revoked_at        timestamp without time zone
);

CREATE TABLE public.jobs
(
    id           uuid                        DEFAULT public.uuid_generate_v4()        NOT NULL,
    owner_id     uuid                                                                 NOT NULL,
    job_type public.job_type NOT NULL,
    status public.job_status DEFAULT 'pending'::public.job_status NOT NULL,
    config       jsonb                       DEFAULT '{}'::jsonb                      NOT NULL,
    result       jsonb                       DEFAULT '{}'::jsonb,
    error_message text,
    retry_count  integer                     DEFAULT 0                                NOT NULL,
    max_retries  integer                     DEFAULT 3                                NOT NULL,
    idempotency_key character varying(255),
    picture_id   uuid,
    claimed_by   text,
    claim_token  uuid,
    created_at   timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    started_at   timestamp without time zone,
    completed_at timestamp without time zone,
    trace_context jsonb
);

CREATE TABLE public.outgoing_shares
(
    id                 uuid                        DEFAULT public.uuid_generate_v4()        NOT NULL,
    owner_id           uuid                                                                 NOT NULL,
    tag_path public.ltree NOT NULL,
    name               character varying(64)                                                NOT NULL,
    message            text,
    recipient_username character varying(255)                                               NOT NULL,
    recipient_instance character varying(255)                                               NOT NULL,
    allow_share_back   boolean                     DEFAULT true                             NOT NULL,
    future             boolean                     DEFAULT true                             NOT NULL,
    allow_exif_edit    boolean                     DEFAULT false                            NOT NULL,
    shareback_of       uuid,
    status public.share_status DEFAULT 'pending'::public.share_status NOT NULL,
    last_error_at      timestamp without time zone,
    next_retry_at      timestamp without time zone,
    created_at         timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    revoked_at         timestamp without time zone
);

CREATE TABLE public.picture_versions
(
    id         uuid NOT NULL,
    picture_id uuid NOT NULL,
    version_number integer NOT NULL,
    file_size  bigint,
    mime_type  character varying(100),
    created_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL
);

CREATE TABLE public.pictures
(
    id                     uuid                        DEFAULT public.uuid_generate_v4()        NOT NULL,
    local_user_id          uuid                                                                 NOT NULL,
    remote_picture_id      character varying(255),
    owner_username         character varying(255),
    owner_instance_domain  character varying(255),
    filename               character varying(1024),
    mime_type              character varying(100),
    file_size              bigint,
    width                  integer,
    height                 integer,
    exif_data              jsonb                       DEFAULT '{}'::jsonb                      NOT NULL,
    metadata               jsonb                       DEFAULT '{}'::jsonb                      NOT NULL,
    deleted_at             timestamp without time zone,
    captured_at            timestamp without time zone,
    ingested_at            timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    updated_at             timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    blurhash               text,
    gps_lat                double precision,
    gps_lng                double precision,
    gps_alt                integer,
    orientation            smallint,
    thumbnails_generated_at timestamp without time zone,
    file_hash              text,
    last_pipeline_run_at   timestamp without time zone,
    exif_sync_status public.picture_exif_sync_status DEFAULT 'synced'::public.picture_exif_sync_status NOT NULL,
    owner_deleted_at       timestamp without time zone,
    owner_purge_at         timestamp without time zone,
    remote_exif_data       jsonb,
    local_exif_overrides   jsonb,
    deleted_reason public.picture_deleted_reason,
    content_hash           text,
    copy_source_owner_username character varying(255),
    copy_source_owner_instance character varying(255),
    copy_source_picture_id character varying(255)
);

CREATE TABLE public.refresh_tokens
(
    id         uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    user_id    uuid                                   NOT NULL,
    token_hash text                                   NOT NULL,
    expires_at timestamp without time zone NOT NULL,
    revoked_at timestamp without time zone,
    created_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    updated_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL
);

CREATE TABLE public.share_announcements
(
    outgoing_share_id uuid                           NOT NULL,
    picture_id        uuid                           NOT NULL,
    picture_token     uuid DEFAULT gen_random_uuid() NOT NULL,
    announced_updated_at timestamp without time zone
);

CREATE TABLE public.tagging_services
(
    id            uuid                        DEFAULT public.uuid_generate_v4()        NOT NULL,
    owner_id      uuid                                                                 NOT NULL,
    service_type public.service_type NOT NULL,
    requires public.ltree[] DEFAULT '{}'::public.ltree[] NOT NULL,
    excludes public.ltree[] DEFAULT '{}'::public.ltree[] NOT NULL,
    enabled       boolean                     DEFAULT true                             NOT NULL,
    "position"    integer                     DEFAULT 0                                NOT NULL,
    last_invalidated_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    last_error_at timestamp without time zone,
    last_error_msg text,
    created_at    timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    updated_at    timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    name          character varying(255)      DEFAULT ''::character varying            NOT NULL,
    config        jsonb                       DEFAULT '{}'::jsonb                      NOT NULL
);

CREATE TABLE public.tags
(
    id         uuid DEFAULT public.uuid_generate_v4() NOT NULL,
    picture_id uuid                                   NOT NULL,
    tag_path public.ltree NOT NULL,
    source public.tag_source DEFAULT 'manual'::public.tag_source NOT NULL,
    source_id  uuid,
    picture_token uuid,
    assigned_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL
);

CREATE TABLE public.user_credentials
(
    user_id uuid NOT NULL,
    password_hash text NOT NULL,
    created_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    updated_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL
);

CREATE TABLE public.user_settings
(
    user_id uuid NOT NULL,
    versioning_mode public.versioning_mode DEFAULT 'none'::public.versioning_mode NOT NULL,
    trash_retention_days integer DEFAULT 30 NOT NULL,
    created_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    updated_at timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL
);

CREATE TABLE public.users
(
    id           uuid                        DEFAULT public.uuid_generate_v4()        NOT NULL,
    username     character varying(255)                                               NOT NULL,
    email        character varying(255)                                               NOT NULL,
    display_name character varying(255)                                               NOT NULL,
    is_admin     boolean                     DEFAULT false                            NOT NULL,
    created_at   timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    updated_at   timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL
);

ALTER TABLE ONLY public.federation_messages
    ADD CONSTRAINT federation_messages_idempotency_key_key UNIQUE (idempotency_key);

ALTER TABLE ONLY public.federation_messages
    ADD CONSTRAINT federation_messages_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.hierarchies
    ADD CONSTRAINT hierarchies_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.incoming_shares
    ADD CONSTRAINT incoming_shares_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_idempotency_key_key UNIQUE (idempotency_key);

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.outgoing_shares
    ADD CONSTRAINT outgoing_shares_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.picture_versions
    ADD CONSTRAINT picture_versions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.pictures
    ADD CONSTRAINT pictures_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT refresh_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.share_announcements
    ADD CONSTRAINT share_announcements_pkey PRIMARY KEY (outgoing_share_id, picture_id);

ALTER TABLE ONLY public.tagging_services
    ADD CONSTRAINT tagging_services_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.tags
    ADD CONSTRAINT tags_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.hierarchies
    ADD CONSTRAINT uq_hierarchy_name UNIQUE (owner_id, name);

ALTER TABLE ONLY public.incoming_shares
    ADD CONSTRAINT uq_incoming_share UNIQUE (recipient_id, sender_username, sender_instance, outgoing_share_id);

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT uq_job_idempotency UNIQUE (owner_id, idempotency_key);

ALTER TABLE ONLY public.picture_versions
    ADD CONSTRAINT uq_picture_version UNIQUE (picture_id, version_number);

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT uq_refresh_token_hash UNIQUE (token_hash);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT uq_user_email UNIQUE (email);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT uq_user_username UNIQUE (username);

ALTER TABLE ONLY public.user_credentials
    ADD CONSTRAINT user_credentials_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY public.user_settings
    ADD CONSTRAINT user_settings_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);

CREATE INDEX idx_federation_messages_direction ON public.federation_messages USING btree (direction);

CREATE INDEX idx_federation_messages_recipient ON public.federation_messages USING btree (recipient_username, recipient_instance);

CREATE INDEX idx_federation_messages_sender ON public.federation_messages USING btree (sender_username, sender_instance);

CREATE INDEX idx_federation_messages_status ON public.federation_messages USING btree (status);

CREATE INDEX idx_federation_messages_type ON public.federation_messages USING btree (message_type);

CREATE INDEX idx_hierarchies_owner ON public.hierarchies USING btree (owner_id);

CREATE INDEX idx_incoming_shares_recipient ON public.incoming_shares USING btree (recipient_id);

CREATE INDEX idx_incoming_shares_sender ON public.incoming_shares USING btree (sender_username, sender_instance);

CREATE INDEX idx_incoming_shares_status ON public.incoming_shares USING btree (status);

CREATE INDEX idx_jobs_created ON public.jobs USING btree (created_at);

CREATE INDEX idx_jobs_owner ON public.jobs USING btree (owner_id);

CREATE INDEX idx_jobs_pending_claim ON public.jobs USING btree (job_type, created_at) WHERE (status = 'pending'::public.job_status);

CREATE INDEX idx_jobs_picture ON public.jobs USING btree (picture_id) WHERE (picture_id IS NOT NULL);

CREATE INDEX idx_jobs_status ON public.jobs USING btree (status);

CREATE INDEX idx_jobs_type ON public.jobs USING btree (job_type);

CREATE INDEX idx_outgoing_shares_owner ON public.outgoing_shares USING btree (owner_id);

CREATE INDEX idx_outgoing_shares_recipient ON public.outgoing_shares USING btree (recipient_username, recipient_instance);

CREATE INDEX idx_outgoing_shares_status ON public.outgoing_shares USING btree (status);

CREATE INDEX idx_outgoing_shares_tag ON public.outgoing_shares USING gist (tag_path);

CREATE INDEX idx_picture_versions_picture ON public.picture_versions USING btree (picture_id);

CREATE INDEX idx_pictures_captured ON public.pictures USING btree (captured_at);

CREATE INDEX idx_pictures_content_hash ON public.pictures USING btree (local_user_id, content_hash) WHERE (content_hash IS NOT NULL);

CREATE INDEX idx_pictures_deleted ON public.pictures USING btree (deleted_at) WHERE (deleted_at IS NOT NULL);

CREATE INDEX idx_pictures_exif ON public.pictures USING gin (exif_data);

CREATE INDEX idx_pictures_exif_pending ON public.pictures USING btree (id) WHERE (exif_sync_status = 'pending'::public.picture_exif_sync_status);

CREATE INDEX idx_pictures_gps ON public.pictures USING btree (gps_lat, gps_lng) WHERE (gps_lat IS NOT NULL);

CREATE INDEX idx_pictures_local_user ON public.pictures USING btree (local_user_id);

CREATE INDEX idx_pictures_metadata ON public.pictures USING gin (metadata);

CREATE INDEX idx_pictures_owned_trashed ON public.pictures USING btree (deleted_at) WHERE ((deleted_at IS NOT NULL) AND (remote_picture_id IS NULL));

CREATE INDEX idx_pictures_pipeline ON public.pictures USING btree (local_user_id, last_pipeline_run_at);

CREATE INDEX idx_pictures_remote_owner ON public.pictures USING btree (owner_username, owner_instance_domain) WHERE (owner_username IS NOT NULL);

CREATE INDEX idx_pictures_user_file_size ON public.pictures USING btree (local_user_id, file_size);

CREATE INDEX idx_pictures_user_filename ON public.pictures USING btree (local_user_id, filename);

CREATE INDEX idx_refresh_tokens_expires ON public.refresh_tokens USING btree (expires_at);

CREATE INDEX idx_refresh_tokens_user ON public.refresh_tokens USING btree (user_id);

CREATE INDEX idx_share_announcements_picture ON public.share_announcements USING btree (picture_id);

CREATE INDEX idx_share_announcements_token ON public.share_announcements USING btree (picture_token);

CREATE INDEX idx_tagging_services_enabled ON public.tagging_services USING btree (enabled);

CREATE INDEX idx_tagging_services_mapping_share ON public.tagging_services USING btree (((config ->> 'incoming_share_id'::text))) WHERE (service_type = 'shared_tag_mapping'::public.service_type);

CREATE INDEX idx_tagging_services_owner ON public.tagging_services USING btree (owner_id);

CREATE INDEX idx_tagging_services_type ON public.tagging_services USING btree (service_type);

CREATE INDEX idx_tags_path ON public.tags USING gist (tag_path);

CREATE INDEX idx_tags_picture ON public.tags USING btree (picture_id);

CREATE INDEX idx_tags_picture_token ON public.tags USING btree (picture_token) WHERE (picture_token IS NOT NULL);

CREATE INDEX idx_tags_source ON public.tags USING btree (source, source_id);

CREATE INDEX idx_user_credentials_user ON public.user_credentials USING btree (user_id);

CREATE INDEX idx_users_email ON public.users USING btree (email);

CREATE INDEX idx_users_username ON public.users USING btree (username);

CREATE UNIQUE INDEX uq_edit_picture_inflight ON public.jobs USING btree (picture_id) WHERE ((job_type = 'edit_picture'::public.job_type) AND
                                                                                            (status = ANY (ARRAY ['pending'::public.job_status, 'processing'::public.job_status])));

CREATE UNIQUE INDEX uq_outgoing_share ON public.outgoing_shares USING btree (owner_id, tag_path, recipient_username, recipient_instance) WHERE (
    status <> ALL (ARRAY ['revoked'::public.share_status, 'tombstoned'::public.share_status]));

CREATE UNIQUE INDEX uq_picture_tag_manual ON public.tags USING btree (picture_id, tag_path) WHERE (source = 'manual'::public.tag_source);

CREATE UNIQUE INDEX uq_picture_tag_source ON public.tags USING btree (picture_id, tag_path, source, source_id) WHERE (source <> 'manual'::public.tag_source);

CREATE UNIQUE INDEX uq_received_picture ON public.pictures USING btree (local_user_id, remote_picture_id) WHERE (remote_picture_id IS NOT NULL);

CREATE TRIGGER update_hierarchies_updated_at
    BEFORE UPDATE
    ON public.hierarchies
    FOR EACH ROW
EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_pictures_updated_at
    BEFORE UPDATE
    ON public.pictures
    FOR EACH ROW
EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_refresh_tokens_updated_at
    BEFORE UPDATE
    ON public.refresh_tokens
    FOR EACH ROW
EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_tagging_services_updated_at
    BEFORE UPDATE
    ON public.tagging_services
    FOR EACH ROW
EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_user_credentials_updated_at
    BEFORE UPDATE
    ON public.user_credentials
    FOR EACH ROW
EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_user_settings_updated_at
    BEFORE UPDATE
    ON public.user_settings
    FOR EACH ROW
EXECUTE FUNCTION public.update_updated_at_column();

CREATE TRIGGER update_users_updated_at
    BEFORE UPDATE
    ON public.users
    FOR EACH ROW
EXECUTE FUNCTION public.update_updated_at_column();

ALTER TABLE ONLY public.federation_messages
    ADD CONSTRAINT federation_messages_incoming_share_id_fkey FOREIGN KEY (incoming_share_id) REFERENCES public.incoming_shares (id) ON DELETE SET NULL;

ALTER TABLE ONLY public.federation_messages
    ADD CONSTRAINT federation_messages_outgoing_share_id_fkey FOREIGN KEY (outgoing_share_id) REFERENCES public.outgoing_shares (id) ON DELETE SET NULL;

ALTER TABLE ONLY public.incoming_shares
    ADD CONSTRAINT fk_incoming_shares_mapping FOREIGN KEY (local_mapping_service_id) REFERENCES public.tagging_services (id) ON DELETE SET NULL;

ALTER TABLE ONLY public.hierarchies
    ADD CONSTRAINT hierarchies_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES public.users (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.incoming_shares
    ADD CONSTRAINT incoming_shares_recipient_id_fkey FOREIGN KEY (recipient_id) REFERENCES public.users (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES public.users (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_picture_id_fkey FOREIGN KEY (picture_id) REFERENCES public.pictures (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.outgoing_shares
    ADD CONSTRAINT outgoing_shares_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES public.users (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.picture_versions
    ADD CONSTRAINT picture_versions_picture_id_fkey FOREIGN KEY (picture_id) REFERENCES public.pictures (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.pictures
    ADD CONSTRAINT pictures_local_user_id_fkey FOREIGN KEY (local_user_id) REFERENCES public.users (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.refresh_tokens
    ADD CONSTRAINT refresh_tokens_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.share_announcements
    ADD CONSTRAINT share_announcements_outgoing_share_id_fkey FOREIGN KEY (outgoing_share_id) REFERENCES public.outgoing_shares (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.tagging_services
    ADD CONSTRAINT tagging_services_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES public.users (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.tags
    ADD CONSTRAINT tags_picture_id_fkey FOREIGN KEY (picture_id) REFERENCES public.pictures (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.user_credentials
    ADD CONSTRAINT user_credentials_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users (id) ON DELETE CASCADE;

ALTER TABLE ONLY public.user_settings
    ADD CONSTRAINT user_settings_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users (id) ON DELETE CASCADE;

