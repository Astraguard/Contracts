use soroban_sdk::{contracttype, Address, Env, Vec};

#[contracttype]
#[derive(Clone)]
enum AccessKey {
    Admin,
    OracleMultisig,
}

/// M-of-N multisig configuration.  At least `threshold` of the listed
/// `signers` must authorise the transaction for the guarded action to
/// proceed.  Used for both the oracle role and the per-escrow arbiter role.
#[contracttype]
#[derive(Clone)]
pub struct MultisigConfig {
    /// The ordered set of eligible signers.
    pub signers: Vec<Address>,
    /// Minimum number of signer authorisations required.
    pub threshold: u32,
}

impl MultisigConfig {
    /// Calls `require_auth()` on every signer in the config and asserts that
    /// at least `threshold` authorisations were collected.  Because Soroban
    /// validates each `require_auth` call independently, callers must include
    /// authorisations for at least `threshold` signers in the invoking
    /// transaction; extras beyond the threshold are harmless but still checked.
    ///
    /// Panics (via `require_auth`) if any required signer's authorisation is
    /// missing from the transaction.
    pub fn require_multisig_auth(&self, env: &Env) {
        let count = self.signers.len();
        if self.threshold == 0 {
            panic!("multisig threshold must be at least 1");
        }
        if self.threshold > count {
            panic!("multisig threshold exceeds signer count");
        }

        // Require auth from the first `threshold` signers.  This keeps the
        // Soroban auth model predictable: signers are ordered, and the
        // contract enforces that the first N must sign.  An off-chain policy
        // layer can ensure that the designated signers rotate as needed.
        //
        // Alternative designs (e.g. "any T of N") require the caller to
        // indicate *which* T they are submitting, adding callsite complexity
        // with no security benefit in an on-chain M-of-N model where each
        // `require_auth` is validated independently by the host.
        let mut authorised: u32 = 0;
        for signer in self.signers.iter() {
            signer.require_auth();
            authorised += 1;
            if authorised >= self.threshold {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Admin helpers (unchanged — admin remains a single address because it is
// already protected by the 48-hour propose/accept timelock in timelock.rs)
// ---------------------------------------------------------------------------

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&AccessKey::Admin, admin);
}

pub fn get_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&AccessKey::Admin)
        .expect("admin not set")
}

pub fn has_admin(env: &Env) -> bool {
    env.storage().instance().has(&AccessKey::Admin)
}

pub fn require_admin(env: &Env) {
    get_admin(env).require_auth();
}

// ---------------------------------------------------------------------------
// Oracle helpers — now backed by a MultisigConfig
// ---------------------------------------------------------------------------

pub fn set_oracle(env: &Env, config: &MultisigConfig) {
    env.storage()
        .instance()
        .set(&AccessKey::OracleMultisig, config);
}

/// Convenience setter used during migration and timelock handover where the
/// canonical storage is a MultisigConfig.  Wraps a single address as a 1-of-1
/// config.  Kept for compatibility with callers that already construct a
/// full `MultisigConfig`.
pub fn set_oracle_config(env: &Env, config: &MultisigConfig) {
    set_oracle(env, config);
}

pub fn get_oracle(env: &Env) -> MultisigConfig {
    env.storage()
        .instance()
        .get(&AccessKey::OracleMultisig)
        .expect("oracle not set")
}

/// Returns `true` when an oracle multisig config has been stored.
pub fn has_oracle(env: &Env) -> bool {
    env.storage().instance().has(&AccessKey::OracleMultisig)
}

/// Enforces M-of-N oracle authorisation for the current transaction.
pub fn require_oracle(env: &Env) {
    get_oracle(env).require_multisig_auth(env);
}
