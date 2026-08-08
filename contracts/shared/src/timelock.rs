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

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger as _};
    use soroban_sdk::{contract, contractimpl, Env};

    // ---------------------------------------------------------------------------
    // Minimal test contract — gives us a real contract instance so that
    // instance storage (where PendingAdmin lives) is accessible.
    // ---------------------------------------------------------------------------
    #[contract]
    struct TimelockTestContract;

    #[contractimpl]
    impl TimelockTestContract {
        pub fn init(env: Env, admin: Address) {
            access::set_admin(&env, &admin);
        }

        pub fn propose(env: Env, candidate: Address) {
            propose_admin_change(&env, candidate);
        }

        pub fn accept(env: Env) -> Address {
            execute_admin_change(&env)
        }

        pub fn propose_oracle_fn(env: Env, candidate: Address) {
            propose_oracle_change(&env, candidate);
        }

        pub fn accept_oracle_fn(env: Env) -> Address {
            execute_oracle_change(&env)
        }

        pub fn current_admin(env: Env) -> Address {
            access::get_admin(&env)
        }
    }

    fn deploy(env: &Env, admin: &Address) -> TimelockTestContractClient<'_> {
        let id = env.register(TimelockTestContract, ());
        let client = TimelockTestContractClient::new(env, &id);
        client.init(admin);
        client
    }

    // ---------------------------------------------------------------------------
    // Admin handover — happy path
    // ---------------------------------------------------------------------------

    #[test]
    fn admin_handover_happy_path() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let client = deploy(&env, &admin);

        let t0: u64 = 1_000_000;
        env.ledger().set_timestamp(t0);

        client.propose(&new_admin);

        // Warp past the 48-hour timelock.
        env.ledger().set_timestamp(t0 + ADMIN_TIMELOCK_SECONDS);

        let returned = client.accept();
        assert_eq!(returned, new_admin);
        assert_eq!(client.current_admin(), new_admin);
    }

    // ---------------------------------------------------------------------------
    // accept before timelock elapsed must panic
    // ---------------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "timelock not elapsed")]
    fn admin_handover_too_early_panics() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let client = deploy(&env, &admin);

        let t0: u64 = 1_000_000;
        env.ledger().set_timestamp(t0);

        client.propose(&new_admin);

        // Still 1 second before the window opens.
        env.ledger().set_timestamp(t0 + ADMIN_TIMELOCK_SECONDS - 1);
        client.accept(); // must panic
    }

    // ---------------------------------------------------------------------------
    // accept with no pending proposal must panic
    // ---------------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "no pending admin change")]
    fn admin_handover_no_pending_panics() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let client = deploy(&env, &admin);

        client.accept(); // nothing proposed — must panic
    }

    // ---------------------------------------------------------------------------
    // A second propose before the first is accepted overwrites the candidate.
    // ---------------------------------------------------------------------------

    #[test]
    fn admin_handover_second_propose_overwrites_first() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let candidate_a = Address::generate(&env);
        let candidate_b = Address::generate(&env);
        let client = deploy(&env, &admin);

        let t0: u64 = 1_000_000;
        env.ledger().set_timestamp(t0);

        client.propose(&candidate_a);

        // Propose again before anyone has called accept — should replace the pending entry.
        env.ledger().set_timestamp(t0 + 60); // a minute later
        client.propose(&candidate_b);

        // Warp past the second proposal's ready_at (t0 + 60 + 172800).
        env.ledger().set_timestamp(t0 + 60 + ADMIN_TIMELOCK_SECONDS);

        let result = client.accept();
        assert_eq!(result, candidate_b);
        assert_eq!(client.current_admin(), candidate_b);
    }

    // ---------------------------------------------------------------------------
    // Oracle handover — happy path (mirrors admin tests)
    // ---------------------------------------------------------------------------

    #[test]
    fn oracle_handover_happy_path() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let new_oracle = Address::generate(&env);
        let client = deploy(&env, &admin);

        let t0: u64 = 1_000_000;
        env.ledger().set_timestamp(t0);

        client.propose_oracle_fn(&new_oracle);

        env.ledger().set_timestamp(t0 + ORACLE_TIMELOCK_SECONDS);
        let returned = client.accept_oracle_fn();
        assert_eq!(returned, new_oracle);
    }

    // ---------------------------------------------------------------------------
    // Oracle accept before timelock elapsed must panic
    // ---------------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "timelock not elapsed")]
    fn oracle_handover_too_early_panics() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let new_oracle = Address::generate(&env);
        let client = deploy(&env, &admin);

        let t0: u64 = 1_000_000;
        env.ledger().set_timestamp(t0);

        client.propose_oracle_fn(&new_oracle);
        env.ledger().set_timestamp(t0 + ORACLE_TIMELOCK_SECONDS - 1);
        client.accept_oracle_fn(); // must panic
    }

    // ---------------------------------------------------------------------------
    // Oracle accept with no pending proposal must panic
    // ---------------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "no pending oracle change")]
    fn oracle_handover_no_pending_panics() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let client = deploy(&env, &admin);

        client.accept_oracle_fn(); // nothing proposed — must panic
    }
}
