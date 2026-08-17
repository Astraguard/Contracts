# Mainnet Launch Readiness Checklist

This document is the single gate for closing the "Mainnet launch" roadmap item.
**Do not deploy to mainnet until every item below is checked off.**

Each section maps to one or more open GitHub issues. Cross-link this document in those
issues so reviewers can see the full picture from any entry point.

---

## 1. External Audit

- [ ] Engage an audit firm with Soroban / Rust experience
- [ ] Provide auditors with: all three contract sources (`escrow`, `insurance-pool`,
  `registry-anchor`), `astraguard-shared`, the README security section, and the
  `ARCHITECTURE.md` (once written)
- [ ] Receive final audit report with no critical or high findings open
- [ ] Publish the audit report (or a summary) in `docs/audit/` before mainnet deploy
- [ ] All medium/low findings triaged — each either fixed or explicitly accepted with
  written rationale

> **Tracking issue:** audit-tracking issue (to be opened)

---

## 2. Multisig for Privileged Roles

The README currently calls this out as a known gap:
> "arbiter, oracle, and each committee member are currently single Stellar addresses,
> not on-chain multisig accounts — a real multisig/timelock behind each of those roles
> is real work before mainnet."

- [ ] `oracle` address replaced by a multisig account (≥ 2-of-N signers)
- [ ] Each `arbiter` address in production escrows is a multisig account or a
  well-documented trusted party with a disclosed key-management policy
- [ ] Each `committee` member address for `insurance-pool` is a multisig account or
  covered by the same policy
- [ ] Key-management runbook documented in `docs/operations.md`
- [ ] Emergency key-rotation procedure tested end-to-end on testnet (uses the
  `propose_admin` → 48-hour wait → `accept_admin` flow on all three contracts)

> **Tracking issue:** multisig issue (to be opened)

---

## 3. TTL Constants Validated Against Mainnet Rent Economics

The README notes:
> "TTL threshold/bump constants in `ttl.rs` are reasonable starting values, not tuned
> against a specific network's rent-fee economics yet."

- [ ] Obtain current mainnet rent-fee schedule from Stellar Core release notes /
  `stellar-cli network status`
- [ ] Calculate worst-case TTL extension cost per call for each contract under peak
  load assumptions
- [ ] Update `MIN_TTL_THRESHOLD`, `TTL_BUMP_AMOUNT`, and instance-level constants in
  `contracts/shared/src/ttl.rs` to values derived from that analysis
- [ ] Verify that no live escrow, claim, or flag record can silently expire between
  typical user interactions (use the escrow timeout as the lower bound)
- [ ] Re-run `cargo test --workspace` with the updated constants; confirm nothing regresses

> **Tracking issue:** TTL issue (to be opened)

---

## 4. Cross-Contract Integration Tests

The `tests/README.md` calls out that cross-contract scenarios are not yet covered:

- [ ] Integration test: full escrow lifecycle against a real Soroban token contract
  (not a mock)
- [ ] Integration test: `insurance-pool` — premium → oracle coverage → claim →
  committee approval → payout, with the real token contract
- [ ] Integration test: `registry-anchor` — anchor, query, supersede, re-query, all in
  one multi-step scenario
- [ ] All integration tests run cleanly on testnet (or Soroban sandbox) in CI
- [ ] `tests/README.md` updated to reflect passing status

> **Tracking issue:** integration-tests issue (to be opened)

---

## 5. Event Emission Migration (`#[contractevent]` macro)

The README flags this as a deliberate follow-up:
> "Migrate event emission from `env.events().publish(...)` to the `#[contractevent]`
> macro — migrating changes the on-chain event encoding, so it's left as a deliberate
> follow-up."

- [ ] All three contracts migrated from `env.events().publish(...)` to the
  `#[contractevent]` macro
- [ ] Event encoding change documented in a `CHANGELOG.md` or release notes so
  downstream indexers can adapt
- [ ] Unit tests updated to assert against the new event shape

> This must land **before** mainnet so indexers and the backend oracle aren't built
> against the deprecated encoding.

---

## 6. Production Deployment

- [ ] `scripts/deploy.sh mainnet <source-account>` run successfully; output recorded in
  `deployments/mainnet.json`
- [ ] Each contract initialized with real mainnet addresses (per README Quick Start §3):
  - [ ] `escrow` — `initialize(admin=<ADMIN_ADDRESS>)`
  - [ ] `insurance-pool` — `initialize(admin, oracle, asset, coverage_ratio_bps,
    committee, approval_threshold)` with live multisig addresses
  - [ ] `registry-anchor` — `initialize(admin, oracle)` with live multisig oracle
- [ ] Contract IDs in `deployments/mainnet.json` verified by querying the network
  (`stellar contract invoke --id <ID> -- get_escrow ...`)
- [ ] `deployments/mainnet.json` committed to this repo (contract IDs are public; no
  secrets are stored here)
- [ ] A post-deploy smoke test run: one small escrow created and released on mainnet to
  confirm the full flow end-to-end

---

## 7. Operational Readiness

- [ ] Monitoring and alerting set up for the oracle address (detect if it goes silent)
- [ ] On-call runbook for the claims committee (what to do if a payout is stuck)
- [ ] Incident-response plan for a compromised oracle or admin key
- [ ] `docs/operations.md` written and reviewed by at least one other team member
- [ ] `SECURITY.md` (responsible-disclosure policy) added to the repo root

---

## Sign-Off

Before merging the "Mainnet launch" PR, each of the following must have explicitly
approved this document in a GitHub review:

| Role | GitHub handle | Approved |
|------|--------------|---------|
| Contract author | | ☐ |
| Auditor (lead) | | ☐ |
| Oracle operator | | ☐ |
| At least one committee member | | ☐ |

---

*Last updated: 2026-08-08. Update this document as items are resolved and cross-link
to the relevant PRs/issues.*
