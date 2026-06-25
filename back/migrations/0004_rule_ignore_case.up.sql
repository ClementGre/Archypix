-- Feature 15: rules carry an explicit per-string-condition `ignore_case` flag instead of the
-- separate `eq_ic` operator, and the case-insensitivity of `contains`/`starts_with`/`ends_with`
-- becomes explicit (it used to be implicit/always-on).
--
-- Rewrite every stored rule predicate (a recursive JSONB and/or/not/field/gps tree):
--   * `eq_ic: X`                       -> `eq: X, ignore_case: true`
--   * `contains|starts_with|ends_with` -> add `ignore_case: true` (preserves prior behaviour)
--
-- The helper lives in `pg_temp` so it is session-local and never lands in the dumped schema.

CREATE FUNCTION pg_temp.rewrite_rule_predicate(j jsonb) RETURNS jsonb AS $$
DECLARE
    result jsonb;
BEGIN
    IF j ? 'and' THEN
        RETURN jsonb_build_object('and', (
            SELECT coalesce(jsonb_agg(pg_temp.rewrite_rule_predicate(e)), '[]'::jsonb)
            FROM jsonb_array_elements(j -> 'and') AS e
        ));
    ELSIF j ? 'or' THEN
        RETURN jsonb_build_object('or', (
            SELECT coalesce(jsonb_agg(pg_temp.rewrite_rule_predicate(e)), '[]'::jsonb)
            FROM jsonb_array_elements(j -> 'or') AS e
        ));
    ELSIF j ? 'not' THEN
        RETURN jsonb_build_object('not', pg_temp.rewrite_rule_predicate(j -> 'not'));
    ELSIF j ? 'field' THEN
        result := j;
        IF result ? 'eq_ic' THEN
            result := (result - 'eq_ic')
                || jsonb_build_object('eq', result -> 'eq_ic', 'ignore_case', true);
        ELSIF (result ? 'contains' OR result ? 'starts_with' OR result ? 'ends_with')
            AND NOT (result ? 'ignore_case') THEN
            result := result || jsonb_build_object('ignore_case', true);
        END IF;
        RETURN result;
    ELSE
        RETURN j; -- gps_bbox / gps_radius / leaf without recursion
    END IF;
END;
$$ LANGUAGE plpgsql;

UPDATE rule_tagging_services
SET predicate = pg_temp.rewrite_rule_predicate(predicate);
