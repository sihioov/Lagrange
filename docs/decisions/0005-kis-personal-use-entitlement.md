# ADR-0005: KIS personal-use entitlement owner attestation

- **Status:** Accepted
- **Date:** 2026-08-21, amended 2026-08-29
- **Decider:** Product owner
- Reference: `kis:owner-attestation:personal-single-user:2026-08-21`
- Data coverage start: 2016-08-29
- Lifecycle: active until the owner explicitly revokes or narrows this decision
- Credential decision: reuse the KIS App Key/App Secret already registered by
  the owner through the existing protected secret path; do not request,
  register, or issue a replacement credential

The owner confirms that they hold the rights required to retrieve, retain, and
use the approved read-only KIS Korean market-data surfaces for this project's
private, personal, single-user operation. The repository may retain technical
support for invited users, but those users are not part of the approved data-use
scope and the existence of that capability must not reopen the KIS entitlement
question.

This attestation is the project's written entitlement decision. Agents and
release reviews must treat the KIS personal-use rights question as resolved and
must not ask the owner to reconfirm it unless the owner reports a revocation or
scope change.

On 2026-08-29 the owner explicitly requested that the fixed ETF11 historical
coverage be extended from the launched 6.5-year beta snapshot to ten years and
authorized the already allowlisted, read-only KIS collection needed to start
that work. The retrospective coverage floor is therefore `2016-08-29`; this is
an expansion of the private single-owner date range only. The exact range,
evidence limits, immutable-batch rule, and fail-closed promotion gates are
recorded in ADR-0006.

This decision does not authorize redistribution, Member-visible KR-derived
surfaces, account access, balances, orders, corrections, cancellations,
executions, order WebSockets, or any live-trading path. It does not contain or
stand in for an App Key, App Secret, access token, account identifier, or other
runtime credential. The deny-by-default KIS read-only allowlist in `AGENTS.md`
continues to apply unchanged.
