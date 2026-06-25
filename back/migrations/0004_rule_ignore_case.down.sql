-- Reverse 0004: collapse the explicit `ignore_case` flag back to the legacy model.
--   * `eq: <string>` + `ignore_case: true`            -> `eq_ic: <string>`
--   * `contains|starts_with|ends_with` + `ignore_case` -> drop the flag (was implicitly on)
--   * any remaining `ignore_case`                       -> dropped

CREATE FUNCTION pg_temp.unrewrite_rule_predicate(j jsonb) RETURNS jsonb AS $$
DECLARE
    result jsonb;
BEGIN
    IF j ? 'and' THEN
        RETURN jsonb_build_object('and', (
            SELECT coalesce(jsonb_agg(pg_temp.unrewrite_rule_predicate(e)), '[]'::jsonb)
            FROM jsonb_array_elements(j -> 'and') AS e
        ));
    ELSIF j ? 'or' THEN
        RETURN jsonb_build_object('or', (
            SELECT coalesce(jsonb_agg(pg_temp.unrewrite_rule_predicate(e)), '[]'::jsonb)
            FROM jsonb_array_elements(j -> 'or') AS e
        ));
    ELSIF j ? 'not' THEN
        RETURN jsonb_build_object('not', pg_temp.unrewrite_rule_predicate(j -> 'not'));
    ELSIF j ? 'field' THEN
        result := j;
        IF (result ? 'ignore_case') AND (result ->> 'ignore_case')::bool
            AND (result ? 'eq') AND jsonb_typeof(result -> 'eq') = 'string' THEN
            result := (result - 'eq' - 'ignore_case')
                || jsonb_build_object('eq_ic', result -> 'eq');
        ELSIF result ? 'ignore_case' THEN
            result := result - 'ignore_case';
        END IF;
        RETURN result;
    ELSE
        RETURN j;
    END IF;
END;
$$ LANGUAGE plpgsql;

UPDATE rule_tagging_services
SET predicate = pg_temp.unrewrite_rule_predicate(predicate);
