-- Restoring the permanent constraints can fail if terminal jobs now share a key — exactly what the
-- up migration set out to allow. Deduplicate by hand before rolling back.
DROP INDEX IF EXISTS uq_jobs_idempotency_live;

ALTER TABLE jobs
    ADD CONSTRAINT jobs_idempotency_key_key UNIQUE (idempotency_key);
ALTER TABLE jobs
    ADD CONSTRAINT uq_job_idempotency UNIQUE (owner_id, idempotency_key);
