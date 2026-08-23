-- A cumulative price generation binds more than one Raw delivery.
--
-- f815f63 added bind_price_batch_to_existing_generation, which binds every
-- older Raw EOD delivery in a cumulative generation to the dataset version its
-- anchor originated, passing reused_existing = true for response kind 'bars'.
-- It did not touch a migration, and the precondition in
-- bind_candidate_raw_dataset still listed only the three kinds that could be
-- reused before price generations existed. So that path has never succeeded:
-- publishing a second session raised 'candidate Raw dataset binding is
-- invalid' while the anchor's own binding, written with reused_existing =
-- false, sat in the same table.
--
-- Adding 'bars' does not loosen the contract. The clause immediately after it
-- already requires, for any reuse, an origin binding on the same dataset
-- version and response kind that is non-reused and belongs to a PUBLISHED
-- batch -- the invariant the allowlist was standing in for.
CREATE OR REPLACE FUNCTION public.bind_candidate_raw_dataset(
    p_batch_id uuid, p_surface text, p_response_kind text,
    p_dataset_version_id uuid, p_reused_existing boolean
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $raw_bind$
DECLARE
    v_dataset_id text;
    v_expected_dataset text;
BEGIN
    SELECT dataset.dataset_id
      INTO v_dataset_id
      FROM public.dataset_versions AS dataset
     WHERE dataset.id = p_dataset_version_id
       AND dataset.status IN ('READY', 'WARNING');
    v_expected_dataset := CASE p_response_kind
        WHEN 'bars' THEN 'krx_eod_bars'
        WHEN 'investor_flow' THEN 'krx_investor_flows'
        WHEN 'market_status' THEN 'krx_market_status'
        WHEN 'fundamentals' THEN 'krx_fundamentals'
        WHEN 'sector_classification' THEN 'krx_sector_classification'
        WHEN 'index_membership' THEN v_dataset_id
        ELSE NULL
    END;
    IF p_surface NOT IN ('source', 'price')
       OR v_dataset_id IS NULL
       OR v_expected_dataset IS NULL
       OR (p_surface = 'source' AND p_response_kind = 'bars')
       OR (p_surface = 'price' AND p_response_kind <> 'bars')
       OR (p_response_kind <> 'index_membership' AND v_dataset_id <> v_expected_dataset)
       OR (p_response_kind = 'index_membership' AND NOT EXISTS (
           SELECT 1 FROM public.candidate_universe_registry AS registry
            WHERE registry.membership_dataset_id = v_dataset_id
              AND registry.enabled
       ))
       -- 'bars' joined this set when cumulative price generations arrived.
       -- One generation is originated by its anchor batch and every older
       -- Raw delivery in the same immutable generation is bound to it as a
       -- reuse, which is exactly what bind_price_batch_to_existing_generation
       -- does. The reuse is still not unchecked: the clause below requires an
       -- origin binding for the same dataset version that is itself
       -- non-reused and PUBLISHED.
       OR (p_reused_existing AND p_response_kind NOT IN (
           'bars', 'fundamentals', 'index_membership', 'sector_classification'
       ))
       OR NOT EXISTS (
           SELECT 1
             FROM public.candidate_raw_batch_publications AS batch
            WHERE batch.batch_id = p_batch_id
              AND batch.surface = p_surface
              AND batch.state IN ('CATALOGED', 'PUBLISHED')
       )
    THEN
        RAISE EXCEPTION 'candidate Raw dataset binding is invalid' USING ERRCODE = '23514';
    END IF;

    IF EXISTS (
        SELECT 1 FROM public.candidate_raw_batch_publications AS batch
         WHERE batch.batch_id = p_batch_id
           AND batch.surface = p_surface
           AND batch.state = 'PUBLISHED'
    ) THEN
        IF EXISTS (
            SELECT 1
              FROM public.candidate_raw_batch_datasets AS binding
             WHERE binding.batch_id = p_batch_id
               AND binding.surface = p_surface
               AND binding.dataset_id = v_dataset_id
               AND binding.response_kind = p_response_kind
               AND binding.dataset_version_id = p_dataset_version_id
               AND binding.reused_existing = p_reused_existing
        ) THEN
            RETURN;
        END IF;
        RAISE EXCEPTION 'published candidate Raw batch cannot gain another dataset binding'
            USING ERRCODE = '23514';
    END IF;

    IF p_reused_existing AND NOT EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS origin
          JOIN public.candidate_raw_batch_publications AS origin_batch
            ON origin_batch.batch_id = origin.batch_id
           AND origin_batch.surface = origin.surface
         WHERE origin.dataset_version_id = p_dataset_version_id
           AND origin.dataset_id = v_dataset_id
           AND origin.response_kind = p_response_kind
           AND NOT origin.reused_existing
           AND origin_batch.state = 'PUBLISHED'
    ) THEN
        RAISE EXCEPTION 'candidate reusable dataset has no sealed immutable origin'
            USING ERRCODE = '23514';
    END IF;
    IF NOT p_reused_existing AND EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS origin
         WHERE origin.dataset_version_id = p_dataset_version_id
           AND NOT origin.reused_existing
           AND (origin.batch_id <> p_batch_id OR origin.surface <> p_surface)
    ) THEN
        RAISE EXCEPTION 'candidate dataset already belongs to another immutable origin'
            USING ERRCODE = '23514';
    END IF;

    INSERT INTO public.candidate_raw_batch_datasets (
        batch_id, surface, response_kind, dataset_id,
        dataset_version_id, reused_existing
    )
    VALUES (
        p_batch_id, p_surface, p_response_kind, v_dataset_id,
        p_dataset_version_id, p_reused_existing
    )
    ON CONFLICT (batch_id, surface, dataset_id) DO NOTHING;
    IF NOT EXISTS (
        SELECT 1
          FROM public.candidate_raw_batch_datasets AS binding
         WHERE binding.batch_id = p_batch_id
           AND binding.surface = p_surface
           AND binding.dataset_id = v_dataset_id
           AND binding.response_kind = p_response_kind
           AND binding.dataset_version_id = p_dataset_version_id
           AND binding.reused_existing = p_reused_existing
    ) THEN
        RAISE EXCEPTION 'candidate Raw dataset binding replay conflicts'
            USING ERRCODE = '23514';
    END IF;
END
$raw_bind$;
