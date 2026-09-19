-- Direction for `children_order` (feature 34 §7): the field says what to sort children by, this
-- says which way round. Ascending keeps today's behaviour for every existing row.
ALTER TABLE public.tag_metadata
    ADD COLUMN children_order_desc boolean NOT NULL DEFAULT false;
