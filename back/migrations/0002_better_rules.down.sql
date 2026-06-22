-- Revert Feature 13 — Better Rules, in reverse order of the up migration.

-- 3. Drop the tagging service name.
ALTER TABLE tagging_services
    DROP COLUMN name;

-- 2. Drop the rule display order.
DROP INDEX IF EXISTS idx_rts_position;
ALTER TABLE rule_tagging_services
    DROP COLUMN position;

-- 1. Revert the rule predicate column to TEXT. The JSON is serialised back to its text form;
-- the original legacy function-call syntax is not reconstructed (best-effort revert).
ALTER TABLE rule_tagging_services
    ALTER COLUMN predicate TYPE text USING predicate::text;
