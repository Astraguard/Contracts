use crate::access;
use soroban_sdk::{contracttype, Address, Env, Symbol};

/// Delay between an admin change being proposed and it taking effect, so
/// admin changes are visible before they land rather than instant.
pub const ADMIN_TIMELOCK_SECONDS: u64 = 172_800; // 48 hours

/// Same delay for oracle rotation: the oracle has unilateral write access
/// to coverage and fraud-flag data, so a key-compromise window should be
/// the same as for admin.
pub const ORACLE_TIMELOCK_SECONDS: u64 = 172_800; // 48 hours

#[contracttype]
#[derive(Clone)]
struct PendingAdmin {
    candidate: Address,
    ready_at: u64,
}

#[contracttype]
#[derive(Clone)]
struct PendingOracle {
    candidate: Address,
    ready_at: u64,
}

#[contracttype]
#[derive(Clone)]
enum TimelockKey {
    PendingAdmin,
    PendingOracle,
}

/// Current admin proposes a successor. Panics via `require_auth` if the
/// caller isn't the current admin.
pub fn propose_admin_change(env: &Env, candidate: Address) {
    access::require_admin(env);

    let ready_at = env.ledger().timestamp() + ADMIN_TIMELOCK_SECONDS;
    env.storage().instance().set(
        &TimelockKey::PendingAdmin,
        &PendingAdmin {
            candidate: candidate.clone(),
            ready_at,
        },
    );

    env.events()
        .publish((Symbol::new(env, "admin_proposed"),), (candidate, ready_at));
}

/// Executes a previously proposed admin change once the timelock has
/// elapsed. Callable by anyone — the delay itself is the safeguard.
pub fn execute_admin_change(env: &Env) -> Address {
    let pending: PendingAdmin = env
        .storage()
        .instance()
        .get(&TimelockKey::PendingAdmin)
        .expect("no pending admin change");

    if env.ledger().timestamp() < pending.ready_at {
        panic!("timelock not elapsed");
    }

    env.storage().instance().remove(&TimelockKey::PendingAdmin);
    access::set_admin(env, &pending.candidate);

    env.events().publish(
        (Symbol::new(env, "admin_changed"),),
        pending.candidate.clone(),
    );

    pending.candidate
}

/// Current admin proposes a new oracle. The same 48-hour timelock as
/// admin handover applies — the oracle has unilateral write access to
/// coverage status and fraud flags, so its rotation should be equally
/// visible before it takes effect.
pub fn propose_oracle_change(env: &Env, candidate: Address) {
    access::require_admin(env);

    let ready_at = env.ledger().timestamp() + ORACLE_TIMELOCK_SECONDS;
    env.storage().instance().set(
        &TimelockKey::PendingOracle,
        &PendingOracle {
            candidate: candidate.clone(),
            ready_at,
        },
    );

    env.events()
        .publish((Symbol::new(env, "oracle_proposed"),), (candidate, ready_at));
}

/// Executes a previously proposed oracle change once the timelock has
/// elapsed. Callable by anyone — the delay itself is the safeguard.
pub fn execute_oracle_change(env: &Env) -> Address {
    let pending: PendingOracle = env
        .storage()
        .instance()
        .get(&TimelockKey::PendingOracle)
        .expect("no pending oracle change");

    if env.ledger().timestamp() < pending.ready_at {
        panic!("timelock not elapsed");
    }

    env.storage()
        .instance()
        .remove(&TimelockKey::PendingOracle);
    access::set_oracle(env, &pending.candidate);

    env.events().publish(
        (Symbol::new(env, "oracle_changed"),),
        pending.candidate.clone(),
    );

    pending.candidate
}
