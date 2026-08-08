-- Reverse of 0020.
DROP INDEX IF EXISTS order_intents_one_intent_per_client_key;
ALTER TABLE order_intents DROP COLUMN IF EXISTS client_key;
