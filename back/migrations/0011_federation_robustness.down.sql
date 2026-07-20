-- Feature 28 rollback.

CREATE TYPE federation_direction AS ENUM ('inbound', 'outbound');
CREATE TYPE federation_message_type AS ENUM ('share_announcement', 'share_revocation', 'picture_update');
CREATE TYPE federation_status AS ENUM ('pending', 'sent', 'delivered', 'failed');

CREATE TABLE federation_messages
(
    id                 uuid                        DEFAULT uuid_generate_v4()               NOT NULL,
    message_type       federation_message_type                                              NOT NULL,
    direction          federation_direction                                                 NOT NULL,
    sender_username    character varying(255),
    sender_instance    character varying(255),
    recipient_username character varying(255),
    recipient_instance character varying(255),
    outgoing_share_id  uuid,
    incoming_share_id  uuid,
    payload            jsonb                       DEFAULT '{}'::jsonb                      NOT NULL,
    status             federation_status           DEFAULT 'pending'::federation_status     NOT NULL,
    created_at         timestamp without time zone DEFAULT (now() AT TIME ZONE 'utc'::text) NOT NULL,
    sent_at            timestamp without time zone,
    delivered_at       timestamp without time zone,
    idempotency_key    text,
    error_message      text,
    retry_count        integer                     DEFAULT 0                                NOT NULL,
    CONSTRAINT federation_messages_pkey PRIMARY KEY (id),
    CONSTRAINT federation_messages_idempotency_key_key UNIQUE (idempotency_key),
    CONSTRAINT federation_messages_incoming_share_id_fkey FOREIGN KEY (incoming_share_id) REFERENCES incoming_shares (id) ON DELETE SET NULL,
    CONSTRAINT federation_messages_outgoing_share_id_fkey FOREIGN KEY (outgoing_share_id) REFERENCES outgoing_shares (id) ON DELETE SET NULL
);

CREATE INDEX idx_federation_messages_direction ON federation_messages USING btree (direction);
CREATE INDEX idx_federation_messages_recipient ON federation_messages USING btree (recipient_username, recipient_instance);
CREATE INDEX idx_federation_messages_sender ON federation_messages USING btree (sender_username, sender_instance);
CREATE INDEX idx_federation_messages_status ON federation_messages USING btree (status);
CREATE INDEX idx_federation_messages_type ON federation_messages USING btree (message_type);

ALTER TABLE pictures
    DROP COLUMN remote_updated_at;
