# Integration Tests

Unit tests for each contract's own lifecycle live alongside its source
(`contracts/<name>/src/test.rs`) and run with:

```bash
cargo test
```

This directory is reserved for cross-contract integration tests — scenarios
that exercise more than one contract in the same `Env`. None exist yet.

Planned additions:
- `escrow_dispute_to_registry.rs` — a disputed escrow resolution that feeds
  evidence into a confirmed `registry-anchor` flag
- `coverage_suspension.rs` — oracle suspending `insurance-pool` coverage
  after `registry-anchor` anchors a flag against a project
