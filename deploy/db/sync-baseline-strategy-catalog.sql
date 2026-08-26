\set ON_ERROR_STOP on

BEGIN;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

SELECT pg_catalog.set_config(
    'lagrange.strategy_catalog.manifest',
    :'catalog_json',
    true
) AS catalog_manifest_set \gset
SELECT pg_catalog.set_config(
    'lagrange.strategy_catalog.sha256',
    :'catalog_sha256',
    true
) AS catalog_sha256_set \gset

DO $catalog_sync$
DECLARE
    v_manifest jsonb;
    v_sha256 text;
    v_changed boolean := false;
    v_entry jsonb;
    v_strategy public.strategies%ROWTYPE;
    v_version public.strategy_versions%ROWTYPE;
    v_schema public.strategy_parameter_schemas%ROWTYPE;
BEGIN
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('lagrange-baseline-strategy-catalog', 1)
    );
    v_manifest := pg_catalog.current_setting(
        'lagrange.strategy_catalog.manifest'
    )::jsonb;
    v_sha256 := pg_catalog.current_setting(
        'lagrange.strategy_catalog.sha256'
    );

    IF v_sha256 !~ '^[0-9a-f]{64}$'
        OR pg_catalog.jsonb_typeof(v_manifest) IS DISTINCT FROM 'object'
        OR v_manifest->>'schema_id' IS DISTINCT FROM 'lagrange-baseline-strategy-catalog'
        OR (v_manifest->>'schema_version')::integer IS DISTINCT FROM 1
        OR (v_manifest->>'released_at_epoch')::bigint IS DISTINCT FROM 1787670000
        OR pg_catalog.jsonb_typeof(v_manifest->'strategies') IS DISTINCT FROM 'array'
        OR pg_catalog.jsonb_array_length(v_manifest->'strategies') IS DISTINCT FROM 5
        OR (
            SELECT pg_catalog.count(*)
            FROM pg_catalog.jsonb_object_keys(v_manifest) AS key
        ) IS DISTINCT FROM 4
        OR EXISTS (
            SELECT 1
            FROM pg_catalog.jsonb_object_keys(v_manifest) AS key
            WHERE key NOT IN (
                'schema_id', 'schema_version', 'released_at_epoch', 'strategies'
            )
        )
    THEN
        RAISE EXCEPTION 'baseline strategy catalog envelope is invalid'
            USING ERRCODE = '22023';
    END IF;

    IF (
        SELECT pg_catalog.array_agg(entry->>'strategy_id' ORDER BY ordinal)
        FROM pg_catalog.jsonb_array_elements(v_manifest->'strategies')
            WITH ORDINALITY AS records(entry, ordinal)
    ) IS DISTINCT FROM ARRAY[
        'buy_and_hold',
        'trend_following',
        'relative_momentum',
        'dual_momentum',
        'inverse_volatility'
    ]::text[]
    THEN
        RAISE EXCEPTION 'baseline strategy catalog identity is invalid'
            USING ERRCODE = '22023';
    END IF;

    FOR v_entry IN
        SELECT entry
        FROM pg_catalog.jsonb_array_elements(v_manifest->'strategies') AS records(entry)
    LOOP
        IF pg_catalog.jsonb_typeof(v_entry) IS DISTINCT FROM 'object'
            OR (
                SELECT pg_catalog.count(*)
                FROM pg_catalog.jsonb_object_keys(v_entry) AS key
            ) IS DISTINCT FROM 12
            OR EXISTS (
                SELECT 1
                FROM pg_catalog.jsonb_object_keys(v_entry) AS key
                WHERE key NOT IN (
                    'strategy_id', 'version', 'display_name', 'description',
                    'risk_description', 'state', 'required_factors',
                    'min_lookback', 'supported_market', 'cadence',
                    'parameter_schema', 'default_parameters'
                )
            )
            OR v_entry->>'strategy_id' !~ '^[a-z][a-z0-9_]{2,63}$'
            OR v_entry->>'version' IS DISTINCT FROM '1.0.0'
            OR v_entry->>'state' IS DISTINCT FROM 'Draft'
            OR v_entry->>'supported_market' IS DISTINCT FROM 'KRX'
            OR v_entry->>'cadence' IS DISTINCT FROM 'daily'
            OR pg_catalog.jsonb_typeof(v_entry->'required_factors') IS DISTINCT FROM 'array'
            OR pg_catalog.jsonb_typeof(v_entry->'parameter_schema') IS DISTINCT FROM 'object'
            OR pg_catalog.jsonb_typeof(v_entry->'default_parameters') IS DISTINCT FROM 'object'
            OR (v_entry->>'min_lookback')::integer < 0
            OR EXISTS (
                SELECT 1
                FROM pg_catalog.jsonb_array_elements(v_entry->'required_factors') AS factor(value)
                WHERE pg_catalog.jsonb_typeof(value) IS DISTINCT FROM 'string'
            )
        THEN
            RAISE EXCEPTION 'baseline strategy catalog record is invalid'
                USING ERRCODE = '22023';
        END IF;

        SELECT * INTO v_strategy
        FROM public.strategies
        WHERE id = v_entry->>'strategy_id'
        FOR UPDATE;
        IF NOT FOUND THEN
            INSERT INTO public.strategies (
                id, display_name, description, risk_description, state
            ) VALUES (
                v_entry->>'strategy_id',
                v_entry->>'display_name',
                v_entry->>'description',
                v_entry->>'risk_description',
                v_entry->>'state'
            );
            v_changed := true;
        ELSIF v_strategy.display_name IS DISTINCT FROM v_entry->>'display_name'
            OR v_strategy.description IS DISTINCT FROM v_entry->>'description'
            OR v_strategy.risk_description IS DISTINCT FROM v_entry->>'risk_description'
            OR v_strategy.state IS DISTINCT FROM v_entry->>'state'
        THEN
            RAISE EXCEPTION 'installed baseline strategy conflicts with immutable catalog'
                USING ERRCODE = '23505';
        END IF;

        SELECT * INTO v_version
        FROM public.strategy_versions
        WHERE strategy_id = v_entry->>'strategy_id'
          AND version = v_entry->>'version'
        FOR UPDATE;
        IF NOT FOUND THEN
            INSERT INTO public.strategy_versions (
                strategy_id, version, required_factors, min_lookback,
                supported_market, cadence
            ) VALUES (
                v_entry->>'strategy_id',
                v_entry->>'version',
                v_entry->'required_factors',
                (v_entry->>'min_lookback')::integer,
                v_entry->>'supported_market',
                v_entry->>'cadence'
            );
            v_changed := true;
        ELSIF v_version.required_factors IS DISTINCT FROM v_entry->'required_factors'
            OR v_version.min_lookback IS DISTINCT FROM (v_entry->>'min_lookback')::integer
            OR v_version.supported_market IS DISTINCT FROM v_entry->>'supported_market'
            OR v_version.cadence IS DISTINCT FROM v_entry->>'cadence'
        THEN
            RAISE EXCEPTION 'installed baseline strategy version conflicts with immutable catalog'
                USING ERRCODE = '23505';
        END IF;

        SELECT * INTO v_schema
        FROM public.strategy_parameter_schemas
        WHERE strategy_id = v_entry->>'strategy_id'
          AND version = v_entry->>'version'
        FOR UPDATE;
        IF NOT FOUND THEN
            INSERT INTO public.strategy_parameter_schemas (
                strategy_id, version, schema_json
            ) VALUES (
                v_entry->>'strategy_id',
                v_entry->>'version',
                v_entry->'parameter_schema'
            );
            v_changed := true;
        ELSIF v_schema.schema_json IS DISTINCT FROM v_entry->'parameter_schema' THEN
            RAISE EXCEPTION 'installed baseline parameter schema conflicts with immutable catalog'
                USING ERRCODE = '23505';
        END IF;
    END LOOP;

    IF v_changed THEN
        PERFORM public.enqueue_auth_audit(
            'strategy-catalog:' || v_sha256 || ':installed',
            'strategy_catalog.installed',
            NULL,
            'strategy_catalog',
            v_sha256,
            'IMMUTABLE_RELEASE_BASELINE',
            (v_manifest->>'released_at_epoch')::bigint
        );
    END IF;
END
$catalog_sync$;

COMMIT;
SELECT 'STRATEGY_CATALOG_SYNC: PASS strategies=5';
