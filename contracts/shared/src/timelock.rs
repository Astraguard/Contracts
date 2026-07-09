use crate::access;
use soroban_sdk::{contracttype, Address, Env, Symbol};

/// Delay between an admin change being proposed and it taking effect, so
/// admin changes are visible before they land rather than instant.
pub const ADMIN_TIMELOCK_SECONDS: u64 = 172_800; // 48 hours

#[contracttype]
#[derive(Clone)]
struct PendingAdmin {
    candidate: Address,
    ready_at: u64,
}

#[contracttype]
#[derive(Clone)]
enum TimelockKey {
    PendingAdmin,
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
