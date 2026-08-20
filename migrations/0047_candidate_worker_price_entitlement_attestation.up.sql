-- Candidate execution re-attests the fixed price dataset under the worker role.
-- Delegate only the read-only Boolean predicate; direct entitlement-table access
-- remains forbidden.
GRANT EXECUTE ON FUNCTION
    public.price_dataset_entitlement_is_valid(uuid, text, date, date)
    TO worker;
