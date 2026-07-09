use soroban_sdk::Env;

// Ledger counts assume ~5s per ledger close. Tune before mainnet against
// the target network's max entry TTL and the rent-fee cost of the bump —
// these values are a reasonable starting point, not a validated constant.
pub const INSTANCE_THRESHOLD_LEDGERS: u32 = 120_960; // ~7 days
pub const INSTANCE_BUMP_LEDGERS: u32 = 518_400; // ~30 days
pub const PERSISTENT_THRESHOLD_LEDGERS: u32 = 120_960; // ~7 days
pub const PERSISTENT_BUMP_LEDGERS: u32 = 518_400; // ~30 days

/// Extends the contract instance's own storage TTL (admin/oracle/config).
/// Call from every state-changing entrypoint so the instance never expires
/// out from under an otherwise-active contract.
pub fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_THRESHOLD_LEDGERS, INSTANCE_BUMP_LEDGERS);
}
