-- Reverse of 0019. Events drop before intents (the FK points that way).

DROP TRIGGER IF EXISTS order_intent_events_append_only ON order_intent_events;
DROP FUNCTION IF EXISTS order_intent_events_reject_mutation();

DROP TABLE IF EXISTS order_intent_events;
DROP TABLE IF EXISTS order_intents;
