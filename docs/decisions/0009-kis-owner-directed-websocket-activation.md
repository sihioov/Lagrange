# ADR-0009: Owner-directed read-only WebSocket activation

- Status: Accepted for the bounded deployment below; live acceptance remains separate
- Date: 2026-10-07
- Decider: Product Owner; deployment interpretation by the coordinator

The Owner instructed realtime quotes to use WebSocket only, accepted the documented
plaintext market connection, and explicitly requested production activation. After the
coordinator reported that the latest capacity notice and other-host use of the same key
were unconfirmed, the Owner again instructed: “배포 진행하라고”. The coordinator applies
this latest instruction as authorization to proceed with the requested bounded read-only
deployment under the disclosed uncertainty, instead of requiring another activation
confirmation. The Owner did not assert a current numeric quota or exclusive global key use.

This is a narrow exception to the earlier G2 pre-activation evidence hold. G2 remains
UNVERIFIED, and this decision does not label a quota or another client's absence as proven.
The existing runtime limits remain one owned market connection and at most thirty desired,
pending or acknowledged registrations, solely H0STCNT0 in the fixed stock universe. Actual
admitted memberships and genuine authenticated Owner demand determine registrations. The
existing READY pilot may receive data without fabricating admission for the remaining stocks.

Conflicting, rejected, rate-limited or ambiguous provider outcomes retain their typed failure
handling and bounded budgets. Do not force-close another client's session, reset approval
state or counters, rotate credentials, add another key, retry an uncertain issuance, silently
reduce the product universe, or introduce REST realtime fallback. The original account,
order, Member delivery and redistribution exclusions remain in force.

Existing ADR-0005 personal single-Owner rights apply only to this market channel, the
ephemeral latest-value cache and authenticated Owner SSE delivery. The reviewed production
grant must still bind the actual Owner, ACTIVE entitlement, existing slot/generation, exact
wire/identity/network pins, dates and clean activation commit. No fixture or manual SQL may
stand in for this binding.

The exact HTTPS Approval request and plaintext market WebSocket endpoint stay unchanged.
Current-day calendar/version/batch lineage, the installed operational session window,
persistent state identity and permissions, packaged binaries, immutable image manifest and
service health must pass their real checks. Missing next-day proof closes collection; this
decision does not invent a future trading day or extend regular market hours.

Keep the installed a68e4dd753271d15f5311e85c1de573e046b7e96 quotes-off release for rollback.
Build and install a different immutable activation release using the mandatory serialized
production workflow, then install the reviewed grant and refresh API, Web and Owner V2 through
their official helpers. Do not edit an installed dotenv or override it from the shell.

Deployment enabled, broker connected, actual receipt observed and LIVE_ACCEPTED are distinct
results. Report each from its evidence. Full live acceptance still requires the outstanding
current-capacity facts, thirty real admissions/ACKs and authenticated delivery/lifecycle
checks; a healthy daemon or a periodic status event is not a fresh quote.
