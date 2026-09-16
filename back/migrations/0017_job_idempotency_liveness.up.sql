-- Make the `jobs` idempotency key actually idempotent (feature 33 §12.2).
--
-- Two overlapping constraints guarded the key: a global `idempotency_key UNIQUE` and the composite
-- `uq_job_idempotency (owner_id, idempotency_key)`. The global one subsumed the composite (dead
-- index) and forced owner scoping into the key string. Worse, both were permanent: a key stayed
-- burned after its job reached a terminal status, until `JobCleanupRoutine` pruned the row — so
-- "don't enqueue while one is in flight" silently meant "don't enqueue again for JOB_RETENTION_SECS".
--
-- Both are replaced by one partial unique index scoped to the non-terminal statuses: the key now
-- means "at most one live job per (owner, key)", and a terminal job frees it. Strictly weaker than
-- what it replaces, so no existing row can violate it.

ALTER TABLE jobs
    DROP CONSTRAINT jobs_idempotency_key_key;
ALTER TABLE jobs
    DROP CONSTRAINT uq_job_idempotency;

CREATE UNIQUE INDEX uq_jobs_idempotency_live
    ON jobs (owner_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL AND status IN ('pending', 'processing');
