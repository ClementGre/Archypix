-- Feature 14 (Better Batch Editing) §5 — deferred EXIF jobs.
--
-- A batch EXIF edit applies a single set-based UPDATE that stamps this transient state instead of
-- enumerating-then-creating one `edit_picture` job per picture. A drain task then selects rows in
-- this state with no in-flight job, creates the reconcile jobs, and flips them to `pending`. The
-- convergence path is therefore `pending_job_creation → pending → synced` (or `unsupported`).
--
-- `ADD VALUE` only *adds* the label (it is never *used* in this migration), so it is safe inside
-- the migration transaction.
ALTER TYPE public.picture_exif_sync_status ADD VALUE IF NOT EXISTS 'pending_job_creation';
