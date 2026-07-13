-- Picture creator attribution (feature 26). Distinct from owner.
-- `creator`          — owner-authoritative credit (NULL ⇒ owner default `@username:global_domain`);
--                       for a received row it is the origin's already-resolved, propagated value.
-- `creator_override` — recipient-local relabel (received pictures only); never propagates.
ALTER TABLE pictures
    ADD COLUMN creator TEXT;
ALTER TABLE pictures
    ADD COLUMN creator_override TEXT;
