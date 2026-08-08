#![no_std]

//! Registry anchor: oracle-only append of confirmed fraud flags. Purpose:
//! public, tamper-evident proof of *when* an address was flagged. The full
//! report (evidence, victim data) is expected to live off-chain; only a
//! hash is anchored here.
//!
//! The anchor's timestamp is taken from `env.ledger().timestamp()` rather
//! than accepted as a caller-supplied argument — a caller-supplied
//! timestamp would let the oracle backdate flags, which defeats the point
//! of anchoring them at all.
//!
//! ## Supersession
//!
//! Flags are append-only and cannot be deleted (tamper-evidence). If the
//! oracle anchors a flag in error — fat-fingered subject, false positive
//! from the off-chain two-person review — the oracle can call
//! `supersede_flag(flag_id, reason_hash)` to attach a `Supersession`
//! record. The original `Flag` record is untouched; the supersession is
//! stored separately under `DataKey::Supersession(flag_id)` and its
//! presence is exposed through `get_flag` (returns the `Flag` as-is) and
//! `get_flag_with_supersession` (returns both together). Consumers of
//! `get_flags_for_subject` should call `get_flag_with_supersession` on
//! each id and filter out any that carry a `Supersession` if they only
//! want live flags.

#[cfg(test)]
mod test;

use astraguard_shared::{access, timelock, ttl};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Symbol, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    FlagNotFound = 2,
    AlreadySuperseded = 3,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlagCategory {
    Scam,
    RugPull,
    Phishing,
    FakeTeam,
    Other,
}

/// An anchored fraud flag. This struct is append-only and never mutated
/// after the initial write; correctness signals are expressed through a
/// separate `Supersession` record stored under `DataKey::Supersession`.
#[contracttype]
#[derive(Clone)]
pub struct Flag {
    pub subject: Address,
    pub record_hash: BytesN<32>,
    pub category: FlagCategory,
    pub anchored_at: u64,
    pub anchored_by: Address,
}

/// Attached to a `Flag` when the oracle determines it was anchored in
/// error (wrong subject, false positive, etc.). The original `Flag` record
/// is preserved for tamper-evidence; the presence of a `Supersession`
/// indicates that consumers should treat the flag as retracted.
///
/// - `reason_hash`: SHA-256 (or equivalent) of the off-chain correction
///   document explaining why this flag is being retracted.
/// - `superseded_at`: ledger timestamp at which the supersession was
///   anchored — taken from the ledger clock, not caller-supplied.
/// - `superseded_by`: oracle address that issued the supersession.
#[contracttype]
#[derive(Clone)]
pub struct Supersession {
    pub reason_hash: BytesN<32>,
    pub superseded_at: u64,
    pub superseded_by: Address,
}

/// Convenience wrapper returned by `get_flag_with_supersession`.
/// `supersession` is `None` when the flag is live, `Some(...)` when
/// it has been retracted by the oracle.
#[contracttype]
#[derive(Clone)]
pub struct FlagWithSupersession {
    pub flag: Flag,
    pub supersession: Option<Supersession>,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    FlagCount,
    Flag(u64),
    SubjectFlags(Address),
    Supersession(u64),
}

#[contract]
pub struct RegistryAnchorContract;

#[contractimpl]
impl RegistryAnchorContract {
    pub fn initialize(env: Env, admin: Address, oracle: Address) -> Result<(), Error> {
        if access::has_admin(&env) {
            return Err(Error::AlreadyInitialized);
        }
        access::set_admin(&env, &admin);
        access::set_oracle(&env, &oracle);
        env.storage().instance().set(&DataKey::FlagCount, &0u64);
        ttl::bump_instance(&env);
        Ok(())
    }

    /// Oracle-only: permanently anchors a confirmed fraud flag against
    /// `subject`. Returns the new flag's id.
    pub fn anchor_flag(
        env: Env,
        subject: Address,
        record_hash: BytesN<32>,
        category: FlagCategory,
    ) -> u64 {
        access::require_oracle(&env);

        let flag_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::FlagCount)
            .unwrap_or(0);
        let flag = Flag {
            subject: subject.clone(),
            record_hash,
            category,
            anchored_at: env.ledger().timestamp(),
            anchored_by: access::get_oracle(&env),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Flag(flag_id), &flag);
        env.storage().persistent().extend_ttl(
            &DataKey::Flag(flag_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        env.storage()
            .instance()
            .set(&DataKey::FlagCount, &(flag_id + 1));
        ttl::bump_instance(&env);

        let mut subject_flags = Self::get_flags_for_subject(env.clone(), subject.clone());
        subject_flags.push_back(flag_id);
        env.storage()
            .persistent()
            .set(&DataKey::SubjectFlags(subject.clone()), &subject_flags);
        env.storage().persistent().extend_ttl(
            &DataKey::SubjectFlags(subject.clone()),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );

        env.events()
            .publish((Symbol::new(&env, "flagged"), subject), flag_id);

        flag_id
    }

    /// Oracle-only: marks a previously anchored flag as superseded
    /// (retracted/corrected) without deleting the original record.
    ///
    /// The original `Flag` entry is left completely untouched — tamper-
    /// evidence is preserved. A `Supersession` record is written under
    /// `DataKey::Supersession(flag_id)` with the ledger timestamp and the
    /// `reason_hash` (hash of the off-chain correction document). Calling
    /// this on a flag that has already been superseded returns
    /// `Error::AlreadySuperseded`.
    ///
    /// Returns the `Supersession` that was written.
    pub fn supersede_flag(
        env: Env,
        flag_id: u64,
        reason_hash: BytesN<32>,
    ) -> Result<Supersession, Error> {
        access::require_oracle(&env);

        // Ensure the flag actually exists.
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Flag(flag_id))
        {
            return Err(Error::FlagNotFound);
        }

        // Idempotency guard — superseding a superseded flag is an error so
        // the oracle can't silently overwrite an existing correction record.
        if env
            .storage()
            .persistent()
            .has(&DataKey::Supersession(flag_id))
        {
            return Err(Error::AlreadySuperseded);
        }

        let supersession = Supersession {
            reason_hash,
            superseded_at: env.ledger().timestamp(),
            superseded_by: access::get_oracle(&env),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Supersession(flag_id), &supersession);
        env.storage().persistent().extend_ttl(
            &DataKey::Supersession(flag_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        ttl::bump_instance(&env);

        env.events().publish(
            (Symbol::new(&env, "flag_superseded"),),
            (flag_id, supersession.superseded_at),
        );

        Ok(supersession)
    }

    /// Returns the raw `Flag` record. The caller must separately check
    /// `get_supersession` (or use `get_flag_with_supersession`) to
    /// determine whether the flag has been retracted.
    pub fn get_flag(env: Env, flag_id: u64) -> Result<Flag, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Flag(flag_id))
            .ok_or(Error::FlagNotFound)
    }

    /// Returns the `Supersession` attached to `flag_id`, or `None` if the
    /// flag is still live. Returns `Error::FlagNotFound` if the flag
    /// itself does not exist.
    pub fn get_supersession(env: Env, flag_id: u64) -> Result<Option<Supersession>, Error> {
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Flag(flag_id))
        {
            return Err(Error::FlagNotFound);
        }
        Ok(env
            .storage()
            .persistent()
            .get(&DataKey::Supersession(flag_id)))
    }

    /// Returns both the `Flag` and its `Supersession` (if any) in a single
    /// call. Preferred over calling `get_flag` and `get_supersession`
    /// separately when building a view that needs to distinguish live flags
    /// from retracted ones.
    pub fn get_flag_with_supersession(
        env: Env,
        flag_id: u64,
    ) -> Result<FlagWithSupersession, Error> {
        let flag: Flag = env
            .storage()
            .persistent()
            .get(&DataKey::Flag(flag_id))
            .ok_or(Error::FlagNotFound)?;

        let supersession: Option<Supersession> = env
            .storage()
            .persistent()
            .get(&DataKey::Supersession(flag_id));

        Ok(FlagWithSupersession { flag, supersession })
    }

    /// Returns all flag ids ever anchored against `subject`, including any
    /// that have since been superseded. Callers that only want live flags
    /// should call `get_flag_with_supersession` on each id and discard
    /// those that carry a `Supersession`.
    pub fn get_flags_for_subject(env: Env, subject: Address) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::SubjectFlags(subject))
            .unwrap_or(Vec::new(&env))
    }

    pub fn propose_admin(env: Env, candidate: Address) {
        timelock::propose_admin_change(&env, candidate);
    }

    pub fn accept_admin(env: Env) -> Address {
        timelock::execute_admin_change(&env)
    }

    /// Admin proposes a new oracle address. The proposal enters a 48-hour
    /// timelock (same as admin handover) before `accept_oracle` can execute
    /// it, giving observers a window to notice and react to a compromised
    /// oracle key before it is formally replaced.
    pub fn propose_oracle(env: Env, candidate: Address) {
        timelock::propose_oracle_change(&env, candidate);
    }

    /// Executes a previously proposed oracle change once the 48-hour
    /// timelock has elapsed. Callable by anyone — the delay is the guard.
    pub fn accept_oracle(env: Env) -> Address {
        timelock::execute_oracle_change(&env)
    }
}
