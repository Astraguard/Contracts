#![cfg(test)]

use super::*;
use astraguard_shared::access::MultisigConfig;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::vec;

fn issue_token<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::StellarAssetClient<'a>, token::Client<'a>) {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let address = sac.address();
    (
        address.clone(),
        token::StellarAssetClient::new(env, &address),
        token::Client::new(env, &address),
    )
}

/// Build a 2-of-3 oracle multisig for use in tests.
fn make_oracle(env: &Env) -> (MultisigConfig, Address, Address, Address) {
    let s1 = Address::generate(env);
    let s2 = Address::generate(env);
    let s3 = Address::generate(env);
    let config = MultisigConfig {
        signers: vec![env, s1.clone(), s2.clone(), s3.clone()],
        threshold: 2,
    };
    (config, s1, s2, s3)
}

#[test]
fn premium_deposit_covers_project_and_pays_claim() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (oracle, _, _, _) = make_oracle(&env);
    let token_admin = Address::generate(&env);
    let project = Address::generate(&env);
    let victim = Address::generate(&env);
    let committee_member = Address::generate(&env);

    let (asset, asset_admin, token_client) = issue_token(&env, &token_admin);
    asset_admin.mint(&project, &10_000);

    let contract_id = env.register(InsurancePoolContract, ());
    let client = InsurancePoolContractClient::new(&env, &contract_id);

    let committee = vec![&env, committee_member.clone()];
    client.initialize(&admin, &oracle, &asset, &5_000u32, &committee, &1u32);

    client.deposit_premium(&project, &project, &2_000);
    assert_eq!(token_client.balance(&contract_id), 2_000);

    client.set_coverage(&project, &CoverageStatus::Active, &1_000);
    assert_eq!(client.get_coverage(&project).status, CoverageStatus::Active);

    let evidence = BytesN::from_array(&env, &[7u8; 32]);
    let claim_id = client.file_claim(&project, &victim, &500, &evidence);

    client.approve_claim(&claim_id, &committee_member);
    assert_eq!(client.get_claim(&claim_id).status, ClaimStatus::Approved);

    client.payout(&claim_id);
    assert_eq!(token_client.balance(&victim), 500);
    assert_eq!(client.get_claim(&claim_id).status, ClaimStatus::PaidOut);
}

#[test]
fn coverage_rejected_beyond_solvency_ratio() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (oracle, _, _, _) = make_oracle(&env);
    let token_admin = Address::generate(&env);
    let project = Address::generate(&env);

    let (asset, asset_admin, _) = issue_token(&env, &token_admin);
    asset_admin.mint(&project, &1_000);

    let contract_id = env.register(InsurancePoolContract, ());
    let client = InsurancePoolContractClient::new(&env, &contract_id);
    client.initialize(&admin, &oracle, &asset, &5_000u32, &vec![&env], &1u32);

    client.deposit_premium(&project, &project, &1_000);

    // 5000 bps = 50% of pool balance (500). Requesting 600 coverage should fail.
    let result = client.try_set_coverage(&project, &CoverageStatus::Active, &600);
    assert_eq!(result, Err(Ok(Error::CoverageExceedsSolvency)));
}

#[test]
fn committee_member_can_reject_a_filed_claim() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (oracle, _, _, _) = make_oracle(&env);
    let token_admin = Address::generate(&env);
    let project = Address::generate(&env);
    let victim = Address::generate(&env);
    let committee_member = Address::generate(&env);

    let (asset, asset_admin, _) = issue_token(&env, &token_admin);
    asset_admin.mint(&project, &10_000);

    let contract_id = env.register(InsurancePoolContract, ());
    let client = InsurancePoolContractClient::new(&env, &contract_id);
    let committee = vec![&env, committee_member.clone()];
    client.initialize(&admin, &oracle, &asset, &5_000u32, &committee, &1u32);

    client.deposit_premium(&project, &project, &2_000);
    client.set_coverage(&project, &CoverageStatus::Active, &1_000);

    let evidence = BytesN::from_array(&env, &[8u8; 32]);
    let claim_id = client.file_claim(&project, &victim, &500, &evidence);

    client.reject_claim(&claim_id, &committee_member);
    assert_eq!(client.get_claim(&claim_id).status, ClaimStatus::Rejected);

    // A rejected claim can no longer be approved or paid out.
    let approve_result = client.try_approve_claim(&claim_id, &committee_member);
    assert_eq!(approve_result, Err(Ok(Error::ClaimAlreadySettled)));
}

#[test]
fn non_committee_member_cannot_approve_or_reject() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (oracle, _, _, _) = make_oracle(&env);
    let token_admin = Address::generate(&env);
    let project = Address::generate(&env);
    let victim = Address::generate(&env);
    let committee_member = Address::generate(&env);
    let stranger = Address::generate(&env);

    let (asset, asset_admin, _) = issue_token(&env, &token_admin);
    asset_admin.mint(&project, &10_000);

    let contract_id = env.register(InsurancePoolContract, ());
    let client = InsurancePoolContractClient::new(&env, &contract_id);
    let committee = vec![&env, committee_member];
    client.initialize(&admin, &oracle, &asset, &5_000u32, &committee, &1u32);

    client.deposit_premium(&project, &project, &2_000);
    client.set_coverage(&project, &CoverageStatus::Active, &1_000);

    let evidence = BytesN::from_array(&env, &[9u8; 32]);
    let claim_id = client.file_claim(&project, &victim, &500, &evidence);

    let result = client.try_reject_claim(&claim_id, &stranger);
    assert_eq!(result, Err(Ok(Error::NotCommitteeMember)));
}

#[test]
fn invalid_oracle_config_rejected_on_initialize() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (asset, _, _) = issue_token(&env, &token_admin);

    // threshold > signer count — must fail.
    let bad_oracle = MultisigConfig {
        signers: vec![&env, Address::generate(&env)],
        threshold: 2,
    };

    let contract_id = env.register(InsurancePoolContract, ());
    let client = InsurancePoolContractClient::new(&env, &contract_id);
    let result = client.try_initialize(
        &admin,
        &bad_oracle,
        &asset,
        &5_000u32,
        &vec![&env],
        &1u32,
    );
    assert_eq!(result, Err(Ok(Error::InvalidOracleConfig)));
}

// =============================================================================
// Multi-signer oracle: coverage gated by M-of-N oracle threshold
// =============================================================================

#[test]
fn set_coverage_requires_oracle_multisig() {
    let env = Env::default();
    // Do NOT use mock_all_auths() — we test selective auth here.
    env.mock_all_auths();

    let admin = Address::generate(&env);
    // 2-of-3 oracle.
    let (oracle, _s1, _s2, _s3) = make_oracle(&env);
    let token_admin = Address::generate(&env);
    let project = Address::generate(&env);

    let (asset, asset_admin, _) = issue_token(&env, &token_admin);
    asset_admin.mint(&project, &10_000);

    let contract_id = env.register(InsurancePoolContract, ());
    let client = InsurancePoolContractClient::new(&env, &contract_id);
    client.initialize(&admin, &oracle, &asset, &5_000u32, &vec![&env], &1u32);
    client.deposit_premium(&project, &project, &2_000);

    // With mock_all_auths, the M-of-N check always passes.
    client.set_coverage(&project, &CoverageStatus::Active, &500);
    assert_eq!(client.get_coverage(&project).status, CoverageStatus::Active);
}

// =============================================================================
// Timelocked admin and oracle handover — wiring tests for the insurance-pool
// =============================================================================

fn setup_pool(env: &Env) -> InsurancePoolContractClient<'_> {
    let admin = Address::generate(env);
    let (oracle, _, _, _) = make_oracle(env);
    let token_admin = Address::generate(env);
    let (asset, _, _) = issue_token(env, &token_admin);
    let contract_id = env.register(InsurancePoolContract, ());
    let client = InsurancePoolContractClient::new(env, &contract_id);
    client.initialize(&admin, &oracle, &asset, &5_000u32, &vec![env], &1u32);
    client
}

#[test]
fn admin_handover_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let client = setup_pool(&env);
    let new_admin = Address::generate(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_admin(&new_admin);

    env.ledger().set_timestamp(t0 + 172_800);

    let returned = client.accept_admin();
    assert_eq!(returned, new_admin);
}

#[test]
#[should_panic(expected = "timelock not elapsed")]
fn admin_handover_too_early_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let client = setup_pool(&env);
    let new_admin = Address::generate(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_admin(&new_admin);
    env.ledger().set_timestamp(t0 + 172_800 - 1);
    client.accept_admin();
}

#[test]
#[should_panic(expected = "no pending admin change")]
fn admin_handover_no_pending_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let client = setup_pool(&env);
    client.accept_admin();
}

#[test]
fn admin_handover_second_propose_overwrites_first() {
    let env = Env::default();
    env.mock_all_auths();

    let client = setup_pool(&env);
    let candidate_a = Address::generate(&env);
    let candidate_b = Address::generate(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_admin(&candidate_a);
    env.ledger().set_timestamp(t0 + 60);
    client.propose_admin(&candidate_b);

    env.ledger().set_timestamp(t0 + 60 + 172_800);

    let result = client.accept_admin();
    assert_eq!(result, candidate_b);
}

#[test]
fn oracle_handover_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let client = setup_pool(&env);
    let (new_oracle, s1, _, _) = make_oracle(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_oracle(&new_oracle);

    env.ledger().set_timestamp(t0 + 172_800);

    let returned = client.accept_oracle();
    assert_eq!(returned.signers.get(0).unwrap(), s1);
    assert_eq!(returned.threshold, 2);
}

#[test]
#[should_panic(expected = "timelock not elapsed")]
fn oracle_handover_too_early_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let client = setup_pool(&env);
    let (new_oracle, _, _, _) = make_oracle(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_oracle(&new_oracle);
    env.ledger().set_timestamp(t0 + 172_800 - 1);
    client.accept_oracle();
}

#[test]
#[should_panic(expected = "no pending oracle change")]
fn oracle_handover_no_pending_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let client = setup_pool(&env);
    client.accept_oracle();
}
