#![cfg(test)]

use super::*;
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

#[test]
fn premium_deposit_covers_project_and_pays_claim() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let oracle = Address::generate(&env);
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
    let oracle = Address::generate(&env);
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
    let oracle = Address::generate(&env);
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
    let oracle = Address::generate(&env);
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
