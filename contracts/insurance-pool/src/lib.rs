#![no_std]

//! Insurance pool: `deposit_premium` brings funds in, `set_coverage` is
//! oracle-multisig-only (intended to be driven by an off-chain trust-score
//! service), `file_claim` registers a claim on-chain, and `payout` requires
//! M-of-N approval from the claims committee. Solvency guard: total active
//! coverage exposure must stay within `coverage_ratio_bps` of the pool's
//! token balance.
//!
//! The `oracle` role is now a `MultisigConfig` — a threshold-of-N set of
//! signers.  All oracle-gated operations (`set_coverage`) require at least
//! `oracle.threshold` of the registered oracle signers to co-sign the
//! transaction.

#[cfg(test)]
mod test;

use astraguard_shared::{access, access::MultisigConfig, timelock, ttl};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, BytesN, Env, Symbol, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    ProjectNotCovered = 2,
    ClaimNotFound = 3,
    InvalidAmount = 4,
    CoverageExceedsSolvency = 5,
    NotCommitteeMember = 6,
    ClaimNotApproved = 7,
    ClaimAlreadySettled = 8,
    InsufficientPoolBalance = 9,
    /// Returned when trying to add a member who is already on the committee.
    AlreadyCommitteeMember = 10,
    /// Returned when trying to remove a member who is not on the committee.
    MemberNotFound = 11,
    /// `payout` was called for a claim whose project's coverage is no longer
    /// `Active` (e.g. the oracle suspended or removed coverage after the claim
    /// was filed or approved). Freeze semantics: suspension blocks any payout
    /// that has not yet reached `PaidOut`, regardless of when the claim was
    /// filed or approved.
    CoverageSuspended = 12,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageStatus {
    None,
    Active,
    Suspended,
}

#[contracttype]
#[derive(Clone)]
pub struct Coverage {
    pub status: CoverageStatus,
    pub amount: i128,
    pub updated_at: u64,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimStatus {
    Filed,
    Approved,
    Rejected,
    PaidOut,
}

#[contracttype]
#[derive(Clone)]
pub struct Claim {
    pub project: Address,
    pub victim: Address,
    pub amount: i128,
    pub evidence_hash: BytesN<32>,
    pub status: ClaimStatus,
    pub approvals: u32,
    pub filed_at: u64,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Asset,
    CoverageRatioBps,
    TotalCoverage,
    Committee,
    ApprovalThreshold,
    ClaimCount,
    Coverage(Address),
    Claim(u64),
    ClaimApproval(u64, Address),
}

#[contract]
pub struct InsurancePoolContract;

#[contractimpl]
impl InsurancePoolContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        oracle: MultisigConfig,
        asset: Address,
        coverage_ratio_bps: u32,
        committee: Vec<Address>,
        approval_threshold: u32,
    ) -> Result<(), Error> {
        if access::has_admin(&env) {
            return Err(Error::AlreadyInitialized);
        }
        if oracle.threshold == 0 || oracle.threshold > oracle.signers.len() {
            return Err(Error::InvalidOracleConfig);
        }
        access::set_admin(&env, &admin);
        access::set_oracle(&env, &oracle);
        env.storage().instance().set(&DataKey::Asset, &asset);
        env.storage()
            .instance()
            .set(&DataKey::CoverageRatioBps, &coverage_ratio_bps);
        env.storage()
            .instance()
            .set(&DataKey::TotalCoverage, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::Committee, &committee);
        env.storage()
            .instance()
            .set(&DataKey::ApprovalThreshold, &approval_threshold);
        env.storage().instance().set(&DataKey::ClaimCount, &0u64);
        ttl::bump_instance(&env);
        Ok(())
    }

    /// Certification fees (or voluntary top-ups) flowing into the pool.
    pub fn deposit_premium(
        env: Env,
        from: Address,
        project: Address,
        amount: i128,
    ) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        from.require_auth();

        let asset = Self::asset(&env);
        token::Client::new(&env, &asset).transfer(&from, env.current_contract_address(), &amount);
        ttl::bump_instance(&env);

        env.events()
            .publish((Symbol::new(&env, "premium"), project), amount);
        Ok(())
    }

    /// Oracle-multisig-only: sets a project's coverage status, driven
    /// off-chain by the trust score service.  Rejects new active coverage
    /// that would push total exposure past the pool's solvency ratio.
    ///
    /// Requires at least `oracle.threshold` of the registered oracle signers
    /// to co-sign the transaction.
    pub fn set_coverage(
        env: Env,
        project: Address,
        status: CoverageStatus,
        amount: i128,
    ) -> Result<(), Error> {
        access::require_oracle(&env);
        if amount < 0 {
            return Err(Error::InvalidAmount);
        }

        let previous = Self::get_coverage(env.clone(), project.clone());
        let mut total_coverage: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalCoverage)
            .unwrap_or(0);
        total_coverage -= previous.amount;

        let new_amount = if status == CoverageStatus::Active {
            amount
        } else {
            0
        };
        let new_total = total_coverage + new_amount;

        if status == CoverageStatus::Active {
            let ratio_bps: u32 = env
                .storage()
                .instance()
                .get(&DataKey::CoverageRatioBps)
                .unwrap_or(0);
            let asset = Self::asset(&env);
            let pool_balance =
                token::Client::new(&env, &asset).balance(&env.current_contract_address());
            let max_coverage = pool_balance * (ratio_bps as i128) / 10_000;
            if new_total > max_coverage {
                return Err(Error::CoverageExceedsSolvency);
            }
        }

        let coverage = Coverage {
            status,
            amount: new_amount,
            updated_at: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::Coverage(project.clone()), &coverage);
        env.storage().persistent().extend_ttl(
            &DataKey::Coverage(project.clone()),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        env.storage()
            .instance()
            .set(&DataKey::TotalCoverage, &new_total);
        ttl::bump_instance(&env);

        env.events().publish(
            (Symbol::new(&env, "coverage"), project),
            (status, new_amount),
        );

        Ok(())
    }

    /// Victim files a claim against a project's active coverage.
    pub fn file_claim(
        env: Env,
        project: Address,
        victim: Address,
        amount: i128,
        evidence_hash: BytesN<32>,
    ) -> Result<u64, Error> {
        victim.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let coverage = Self::get_coverage(env.clone(), project.clone());
        if coverage.status != CoverageStatus::Active {
            return Err(Error::ProjectNotCovered);
        }

        let claim_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ClaimCount)
            .unwrap_or(0);
        let claim = Claim {
            project: project.clone(),
            victim,
            amount,
            evidence_hash,
            status: ClaimStatus::Filed,
            approvals: 0,
            filed_at: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Claim(claim_id), &claim);
        env.storage().persistent().extend_ttl(
            &DataKey::Claim(claim_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        env.storage()
            .instance()
            .set(&DataKey::ClaimCount, &(claim_id + 1));
        ttl::bump_instance(&env);

        env.events().publish(
            (Symbol::new(&env, "claim_filed"), claim_id),
            (project, amount),
        );

        Ok(claim_id)
    }

    /// Claims-committee member approval. Once `approval_threshold` distinct
    /// members have approved, the claim becomes payable.
    pub fn approve_claim(env: Env, claim_id: u64, member: Address) -> Result<(), Error> {
        member.require_auth();
        if !Self::is_committee_member(&env, &member) {
            return Err(Error::NotCommitteeMember);
        }

        let mut claim = Self::get_claim(env.clone(), claim_id)?;
        if claim.status != ClaimStatus::Filed {
            return Err(Error::ClaimAlreadySettled);
        }

        let approval_key = DataKey::ClaimApproval(claim_id, member);
        if env.storage().persistent().has(&approval_key) {
            return Ok(());
        }
        env.storage().persistent().set(&approval_key, &true);
        env.storage().persistent().extend_ttl(
            &approval_key,
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        claim.approvals += 1;

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ApprovalThreshold)
            .unwrap_or(1);
        if claim.approvals >= threshold {
            claim.status = ClaimStatus::Approved;
        }

        env.storage()
            .persistent()
            .set(&DataKey::Claim(claim_id), &claim);
        env.storage().persistent().extend_ttl(
            &DataKey::Claim(claim_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        ttl::bump_instance(&env);
        Ok(())
    }

    /// Claims-committee member rejection. A single member can reject a
    /// filed claim (rejecting moves no funds, so it doesn't need the same
    /// M-of-N bar as approval) — this keeps claims from sitting `Filed`
    /// forever when the committee agrees it's invalid.
    pub fn reject_claim(env: Env, claim_id: u64, member: Address) -> Result<(), Error> {
        member.require_auth();
        if !Self::is_committee_member(&env, &member) {
            return Err(Error::NotCommitteeMember);
        }

        let mut claim = Self::get_claim(env.clone(), claim_id)?;
        if claim.status != ClaimStatus::Filed {
            return Err(Error::ClaimAlreadySettled);
        }

        claim.status = ClaimStatus::Rejected;
        env.storage()
            .persistent()
            .set(&DataKey::Claim(claim_id), &claim);
        env.storage().persistent().extend_ttl(
            &DataKey::Claim(claim_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );
        ttl::bump_instance(&env);

        env.events()
            .publish((Symbol::new(&env, "claim_rejected"), claim_id), member);

        Ok(())
    }

    /// Disburses an approved claim from pooled capital.
    ///
    /// Implements **Freeze semantics**: coverage status is re-checked at
    /// payout time, not just at `file_claim`. If the oracle has suspended or
    /// removed the project's coverage since the claim was filed or approved
    /// (e.g. after a fraud flag is confirmed via the registry-anchor), this
    /// call returns `Error::CoverageSuspended` and no funds move. The claim
    /// remains in `Approved` state so the committee can re-evaluate once
    /// coverage is restored, or reject it explicitly.
    pub fn payout(env: Env, claim_id: u64) -> Result<(), Error> {
        let mut claim = Self::get_claim(env.clone(), claim_id)?;
        if claim.status != ClaimStatus::Approved {
            return Err(Error::ClaimNotApproved);
        }

        // Freeze semantics: re-check coverage status at payout time.
        // A suspended or removed project must not pay out claims that haven't
        // already reached PaidOut, even if those claims were filed or approved
        // while coverage was still Active.
        let coverage = Self::get_coverage(env.clone(), claim.project.clone());
        if coverage.status != CoverageStatus::Active {
            return Err(Error::CoverageSuspended);
        }

        let asset = Self::asset(&env);
        let client = token::Client::new(&env, &asset);
        let pool_balance = client.balance(&env.current_contract_address());
        if pool_balance < claim.amount {
            return Err(Error::InsufficientPoolBalance);
        }

        // State is settled before the token transfer (checks-effects-interactions):
        // a panic mid-transfer rolls back the whole invocation in Soroban, so this
        // is safe on the success path and closes a reentrancy window if the pool's
        // asset ever calls back into this contract.
        claim.status = ClaimStatus::PaidOut;
        env.storage()
            .persistent()
            .set(&DataKey::Claim(claim_id), &claim);
        env.storage().persistent().extend_ttl(
            &DataKey::Claim(claim_id),
            ttl::PERSISTENT_THRESHOLD_LEDGERS,
            ttl::PERSISTENT_BUMP_LEDGERS,
        );

        let mut total_coverage: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalCoverage)
            .unwrap_or(0);
        total_coverage = (total_coverage - claim.amount).max(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalCoverage, &total_coverage);
        ttl::bump_instance(&env);

        client.transfer(
            &env.current_contract_address(),
            &claim.victim,
            &claim.amount,
        );

        env.events()
            .publish((Symbol::new(&env, "claim_paid"), claim_id), claim.amount);

        Ok(())
    }

    pub fn get_coverage(env: Env, project: Address) -> Coverage {
        env.storage()
            .persistent()
            .get(&DataKey::Coverage(project))
            .unwrap_or(Coverage {
                status: CoverageStatus::None,
                amount: 0,
                updated_at: 0,
            })
    }

    pub fn get_claim(env: Env, claim_id: u64) -> Result<Claim, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Claim(claim_id))
            .ok_or(Error::ClaimNotFound)
    }

    pub fn pool_balance(env: Env) -> i128 {
        let asset = Self::asset(&env);
        token::Client::new(&env, &asset).balance(&env.current_contract_address())
    }

    pub fn propose_admin(env: Env, candidate: Address) {
        timelock::propose_admin_change(&env, candidate);
    }

    pub fn accept_admin(env: Env) -> Address {
        timelock::execute_admin_change(&env)
    }

    /// Admin proposes a new oracle multisig config. The proposal enters a
    /// 48-hour timelock (same as admin handover) before `accept_oracle` can
    /// execute it, giving observers a window to notice and react to a
    /// compromised oracle key before it is formally replaced.
    pub fn propose_oracle(env: Env, candidate: MultisigConfig) {
        timelock::propose_oracle_change(&env, candidate);
    }

    /// Executes a previously proposed oracle change once the 48-hour
    /// timelock has elapsed. Callable by anyone — the delay is the guard.
    /// Returns the newly active `MultisigConfig`.
    pub fn accept_oracle(env: Env) -> MultisigConfig {
        timelock::execute_oracle_change(&env)
    }

    /// Admin-only: add a new member to the claims committee. Errors if the
    /// address is already a member (idempotency guard).
    pub fn add_committee_member(env: Env, member: Address) -> Result<(), Error> {
        access::require_admin(&env);
        let mut committee: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Committee)
            .unwrap_or(Vec::new(&env));
        if committee.iter().any(|m| &m == &member) {
            return Err(Error::AlreadyCommitteeMember);
        }
        committee.push_back(member.clone());
        env.storage()
            .instance()
            .set(&DataKey::Committee, &committee);
        ttl::bump_instance(&env);

        env.events()
            .publish((Symbol::new(&env, "committee_added"),), member);
        Ok(())
    }

    /// Admin-only: remove a member from the claims committee. Errors if the
    /// address is not currently a member.
    pub fn remove_committee_member(env: Env, member: Address) -> Result<(), Error> {
        access::require_admin(&env);
        let committee: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Committee)
            .unwrap_or(Vec::new(&env));
        let mut updated: Vec<Address> = Vec::new(&env);
        let mut found = false;
        for m in committee.iter() {
            if &m == &member {
                found = true;
            } else {
                updated.push_back(m);
            }
        }
        if !found {
            return Err(Error::MemberNotFound);
        }
        env.storage()
            .instance()
            .set(&DataKey::Committee, &updated);
        ttl::bump_instance(&env);

        env.events()
            .publish((Symbol::new(&env, "committee_removed"),), member);
        Ok(())
    }

    fn asset(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Asset)
            .expect("not initialized")
    }

    fn is_committee_member(env: &Env, member: &Address) -> bool {
        let committee: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Committee)
            .unwrap_or(Vec::new(env));
        committee.iter().any(|m| &m == member)
    }
}
