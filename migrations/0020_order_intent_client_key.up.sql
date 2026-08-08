-- 0020 The client's idempotency key, and the duplicate order it prevents.
-- FR-LIVE-003: "주문 요청에 멱등성 키를 부여해야 한다. 타임아웃 후 같은 의도를
-- 재전송해도 중복 주문이 발생하지 않는다." Plan Todo 39.
--
-- 0019 made `intent_ref` server-generated and globally unique, which is right
-- for the reasons stated there. On its own, though, it left the requirement
-- above UNMET, and in the most dangerous possible way. Trace AT-09:
--
--   1. the client POSTs an order and the response times out;
--   2. the client cannot know whether the order was placed, so it retransmits;
--   3. the route mints a NEW intent_ref, because the ref is server-generated;
--   4. the claim finds nothing to deduplicate against and reports Created;
--   5. the gate runs again -- legitimately, this is a different intent -- and
--      approves;
--   6. a SECOND real order reaches the broker.
--
-- Nothing in the state machine can prevent this: the two submissions are two
-- distinct intents and each is individually legal at every step. The dedup
-- has to happen on the identity the CLIENT controls and repeats, which is the
-- Idempotency-Key header every mutating route already requires.
--
-- Scoped per owner rather than globally: the key is the client's to choose,
-- and one user must not be able to block another's order -- or discover that
-- it exists -- by guessing a key.

ALTER TABLE order_intents
    ADD COLUMN client_key text;

CREATE UNIQUE INDEX order_intents_one_intent_per_client_key
    ON order_intents (owner_user_id, client_key)
    WHERE client_key IS NOT NULL;

COMMENT ON COLUMN order_intents.client_key IS
    'The client Idempotency-Key this intent was created for. A retransmission '
    'carrying the same key resolves to this same intent instead of minting a '
    'new one and placing a second order (FR-LIVE-003).';
