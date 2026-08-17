#![no_std]

//! Conditional payment escrow: `create` locks funds, `release` settles to
//! the seller (buyer-confirmed or after timeout), `dispute` freezes an
//! active escrow, and `resolve` is the arbiter multisig's binding decision.
//!
//! The `arbiter` is now a `MultisigConfig` — a threshold-of-N set of
//! signers that must all authorise the `resolve` call, replacing the
//! original single-address model.  This mirrors the insurance-pool
//! committee's M-of-N approval pattern, generalised to dispute resolution.
//!
//! A future iteration could lean on Stellar-native claimable balances
//! instead of holding funds in contract storage directly; this version uses
//! a plain token-transfer hold, which is simpler to reason about for the MVP.

#[cfg(test)]
mod test;

use astraguard_shared::{access, access::MultisigConfig, timelock, ttl};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, BytesN, Env, Symbol,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    EscrowNotFound = 2,
    InvalidAmount = 3,
    InvalidTimeout = 4,
    NotParty = 5,
    AlreadySettled = 6,
    NotDisputed = 7,
    InvalidSplit = 8,
    /// The arbiter multisig config is invalid (e.g. threshold > signer count
    /// or threshold is zero).
    InvalidArbiterConfig = 9,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    Active,
    Disputed,
    Released,
    Resolved,
}

#[contracttype]
#[derive(Clone)]
pub struct Escrow {
    pub buyer: Address,
    pub seller: Address,
    /// M-of-N arbiter multisig.  At least `arbiter.threshold` of the listed
    /// signers must co-sign the `resolve` transaction.
    pub arbiter: MultisigConfig,
    pub asset: Address,
    pub amount: i128,
    pub timeout: u64,
    /// Hash of the off-chain condition document (e.g. delivery confirmation
    /// terms). Full text is kept off-chain; only its hash is anchored here.
    pub conditions: BytesN<32>,
    pub status: EscrowStatus,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    ReleaseToSeller,
    RefundToBuyer,
    /// Seller's share in basis points (0-10000); the remainder refunds the buyer.
    Split(u32),
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    EscrowCount,
    Escrow(u64),
}

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if access::has_admin(&env) {
            return Err(Error::AlreadyInitialized);
        }
        access::set_admin(&env, &admin);
        env.storage().instance().set(&DataKey::EscrowCount, &0u64);
        ttl::bump_instance(&env);
        Ok(())
    }

    /// Locks `amount` of `asset` from `buyer` under `conditions`, refereed by
    /// the `arbiter` multisig if a dispute is raised.  Returns the new
    /// escrow's id.
    ///
    /// `arbiter` is a `MultisigConfig` — a threshold-of-N set of signers.
    /// Passing a config with `threshold = 1` and a single signer preserves
    /// the original single-address behaviour.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        env: Env,
        buyer: Address,
        seller: Address,
        arbiter: MultisigConfig,
        asset: Address,
        amount: i128,
        timeout: u64,
        conditions: BytesN<32>,
    ) -> Result<u64, Error> {
        buyer.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if timeout <= env.ledger().timestamp() {
            return Err(Error::InvalidTimeout);
        }
        if arbiter.threshold == 0 || arbiter.threshold > arbiter.signers.len() {
            return Err(Error::InvalidArbiterConfig);
        }

        token::Client::new(&env, &asset).transfer(&buyer, env.current_contract_address(), &amount);

        let escrow_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::EscrowCount)
            .unwrap_or(0);

        let escrow = Escrow {
            buyer: buyer.clone(),
            seller: seller.clone(),
            arbiter,
            asset,
            amount,
            timeout,
            conditions,
            status: EscrowStatus::Active,
            created_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
        env.storage().persistent().extend_ttl(
            &DataKey::Escrow(escrow_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        env.storage()
            .instance()
            .set(&DataKey::EscrowCount, &(escrow_id + 1));
        ttl::bump_instance(&env);

        env.events().publish(
            (Symbol::new(&env, "created"), escrow_id),
            (buyer, seller, amount),
        );

        Ok(escrow_id)
    }

    /// Settles funds to the seller. The buyer may call this any time to
    /// confirm early; after `timeout` it's permissionless (funds go to the
    /// rightful party regardless of who submits the transaction).
    pub fn release(env: Env, escrow_id: u64) -> Result<(), Error> {
        let mut escrow = Self::get_escrow(env.clone(), escrow_id)?;

        if escrow.status != EscrowStatus::Active {
            return Err(Error::AlreadySettled);
        }

        if env.ledger().timestamp() < escrow.timeout {
            escrow.buyer.require_auth();
        }

        // State is settled before the token transfer (checks-effects-interactions):
        // a panic mid-transfer rolls back the whole invocation in Soroban, so this
        // is safe on the success path and closes a reentrancy window if `asset`
        // ever calls back into this contract.
        escrow.status = EscrowStatus::Released;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
        env.storage().persistent().extend_ttl(
            &DataKey::Escrow(escrow_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        ttl::bump_instance(&env);

        token::Client::new(&env, &escrow.asset).transfer(
            &env.current_contract_address(),
            &escrow.seller,
            &escrow.amount,
        );

        env.events()
            .publish((Symbol::new(&env, "released"), escrow_id), escrow.amount);

        Ok(())
    }

    /// Either the buyer or the seller may freeze an active escrow pending
    /// arbitration.
    pub fn dispute(
        env: Env,
        escrow_id: u64,
        caller: Address,
        reason: BytesN<32>,
    ) -> Result<(), Error> {
        let mut escrow = Self::get_escrow(env.clone(), escrow_id)?;

        if escrow.status != EscrowStatus::Active {
            return Err(Error::AlreadySettled);
        }
        if caller != escrow.buyer && caller != escrow.seller {
            return Err(Error::NotParty);
        }
        caller.require_auth();

        escrow.status = EscrowStatus::Disputed;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
        env.storage().persistent().extend_ttl(
            &DataKey::Escrow(escrow_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        ttl::bump_instance(&env);

        env.events()
            .publish((Symbol::new(&env, "disputed"), escrow_id), reason);

        Ok(())
    }

    /// Arbiter multisig binding resolution of a disputed escrow.
    ///
    /// The transaction must carry `require_auth` from at least
    /// `escrow.arbiter.threshold` of the registered arbiter signers.  This
    /// is the M-of-N enforcement — a single compromised arbiter key is
    /// insufficient to resolve a dispute unilaterally.
    pub fn resolve(env: Env, escrow_id: u64, decision: Decision) -> Result<(), Error> {
        let mut escrow = Self::get_escrow(env.clone(), escrow_id)?;

        if escrow.status != EscrowStatus::Disputed {
            return Err(Error::NotDisputed);
        }

        // Enforce M-of-N arbiter authorisation.
        escrow.arbiter.require_multisig_auth(&env);

        // Split share is computed (and validated) up front so the state mutation
        // below can't be followed by an error return.
        let split_shares = match decision {
            Decision::Split(seller_bps) => {
                if seller_bps > 10_000 {
                    return Err(Error::InvalidSplit);
                }
                let seller_share = escrow.amount * (seller_bps as i128) / 10_000;
                let buyer_share = escrow.amount - seller_share;
                Some((seller_share, buyer_share))
            }
            _ => None,
        };

        // State is settled before the token transfer(s) (checks-effects-interactions):
        // a panic mid-transfer rolls back the whole invocation in Soroban, so this
        // is safe on the success path and closes a reentrancy window if `asset`
        // ever calls back into this contract.
        escrow.status = EscrowStatus::Resolved;
        env.storage()
            .persistent()
            .set(&DataKey::Escrow(escrow_id), &escrow);
        env.storage().persistent().extend_ttl(
            &DataKey::Escrow(escrow_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        ttl::bump_instance(&env);

        let client = token::Client::new(&env, &escrow.asset);
        let contract_address = env.current_contract_address();

        match decision {
            Decision::ReleaseToSeller => {
                client.transfer(&contract_address, &escrow.seller, &escrow.amount);
            }
            Decision::RefundToBuyer => {
                client.transfer(&contract_address, &escrow.buyer, &escrow.amount);
            }
            Decision::Split(_) => {
                let (seller_share, buyer_share) = split_shares.unwrap();
                if seller_share > 0 {
                    client.transfer(&contract_address, &escrow.seller, &seller_share);
                }
                if buyer_share > 0 {
                    client.transfer(&contract_address, &escrow.buyer, &buyer_share);
                }
            }
        }

        env.events()
            .publish((Symbol::new(&env, "resolved"), escrow_id), ());

        Ok(())
    }

    pub fn get_escrow(env: Env, escrow_id: u64) -> Result<Escrow, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Escrow(escrow_id))
            .ok_or(Error::EscrowNotFound)
    }

    pub fn propose_admin(env: Env, candidate: Address) {
        timelock::propose_admin_change(&env, candidate);
    }

    pub fn accept_admin(env: Env) -> Address {
        timelock::execute_admin_change(&env)
    }
}
