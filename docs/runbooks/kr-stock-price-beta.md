# KR stock price beta materialization

This owner-only, provider-free step reads one explicitly named immutable Raw batch and writes a descriptor-safe daily-bars artifact plus a price/volume snapshot. It has no network, credential, database, Curated, or publication path.

Use exact Raw root, batch UUID, capture commit, checked-in universe and entitlement bytes, and a separate artifact root. `check` needs an owner-reviewed registry; the checked-in empty registry approves nothing. `proposal` writes canonical JSON only outside Git for later owner review.

`RawStore` takes the data root and appends `raw/` itself. Inside the one-shot containers the data root is therefore `/data`, while the host Raw directory is mounted at `/data/raw`. Passing `/data/raw` to the materializer would incorrectly look below `/data/raw/raw` and must fail the static/self-test contract.

The result remains `OWNER_ONLY`, `vendor_snapshot=true`, `strict_pit=false`, `PRICE_VOLUME_RESEARCH_ONLY`, `CONFIGURED_FIXED_LIST`, `NOT_EVALUATED`, and `NOT_PUBLISHED`; it makes no exchange-session, listing, index-membership, redistribution, or adjusted-return claim.
