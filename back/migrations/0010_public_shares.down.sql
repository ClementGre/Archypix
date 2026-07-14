ALTER TABLE outgoing_shares
    DROP COLUMN derived_from_public_share_id;
DROP TABLE public_shares;
DROP TYPE public_share_status;
