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

#[contracttype]
#[derive(Clone)]
pub struct Flag {
    pub subject: Address,
    pub record_hash: BytesN<32>,
    pub category: FlagCategory,
    pub anchored_at: u64,
    pub anchored_by: Address,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    FlagCount,
    Flag(u64),
    SubjectFlags(Address),
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

    pub fn get_flag(env: Env, flag_id: u64) -> Result<Flag, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Flag(flag_id))
            .ok_or(Error::FlagNotFound)
    }

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
}
