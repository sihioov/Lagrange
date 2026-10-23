# R2 I0d-C01 buffered WebSocket fixture correction

The [I0d CI failure](2026-10-15-r2-bootstrap-negative-ci.md) is preserved.
Its 83 bootstrap cases passed; the existing incomplete-frame transport test
received Closed at line882. Static inspection identifies a test-server lifetime
race. CI does not expose the exact thread schedule, so this report distinguishes
the observed Closed failure from the source-based timing explanation.

Only [market_stream_transport.rs](../../crates/kis-client/tests/market_stream_transport.rs)
changes executable source. The shared session_with_buffered_followup fixture and
its four call sites now use three one-shot signals:

1. The client completes subscribe before requesting the packed frame/follow-up.
2. The server acknowledges its single write before the helper returns.
3. The server retains the socket until the client finishes its existing assertions
   and drops the session, then explicitly releases the fixture.

Each receive has a five-second failure bound. Progress depends on acknowledgments,
not elapsed sleeps; the former 25/50/200ms sleeps and 100ms read-based close are
removed from this helper. Dropped senders terminate their waiting receiver with
an error, and successful callers await both fixture tasks. Synthetic frame bytes,
packed ordering, incomplete header, receipt/heartbeat assertions and all poison
assertions are unchanged. The server need not consume the tiny echoed heartbeat
to retain its connection. No production transport, timeout policy, provider path,
suite serialization, ignored test, dependency or workflow changes occur.

The affected named tests are buffered_malformed_wrong_and_late_ack_variants_poison_before_receipts,
buffered_pingpong_and_incomplete_following_frame_preserve_prior_receipts, and
bounded_idle_poison_scan_precedes_queued_receipts. The malformed/wrong/late test
still executes its three distinct inputs through assert_buffered_text_poison.

Coordinator source disposition: ready for one new exact-candidate hosted run.
All function names and four helper calls are retained. Scoped standalone
edition-2024 rustfmt and whitespace checks pass. These are source checks only;
no local compiler, lint, tests, project helper, DB, Docker or provider execution,
and no further worker delegation. This is not a new independent review.

The new run must directly pass these three transport names, the existing 99 R2
Rust names, six separate chart names and 83 bootstrap cases, plus strict lint,
format, PG59, cleanup and every CI job. Unprinted nested helper rows/exits remain
UNKNOWN. Original failed runs are never rerun/cancelled. R2-04/09, 0060,
provider/copy admission, I6, runtime/Owner and final C_R2 remain open.
