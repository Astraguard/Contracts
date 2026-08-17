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

fn setup(env: &Env) -> (Address, EscrowContractClient<'_>) {
    let admin = Address::generate(env);
    let contract_id = env.register(EscrowContract, ());
    let client = EscrowContractClient::new(env, &contract_id);
    client.initialize(&admin);
    (contract_id, client)
}

/// Build a 2-of-3 arbiter multisig for use in tests.
fn make_arbiter(env: &Env) -> (MultisigConfig, Address, Address, Address) {
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
fn create_and_release_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let token_admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let (arbiter, _, _, _) = make_arbiter(&env);

    let (asset, asset_admin, token_client) = issue_token(&env, &token_admin);
    asset_admin.mint(&buyer, &1_000);

    let conditions = BytesN::from_array(&env, &[0u8; 32]);
    let timeout = env.ledger().timestamp() + 3600;

    let escrow_id = client.create(
        &buyer,
        &seller,
        &arbiter,
        &asset,
        &500,
        &timeout,
        &conditions,
    );
    assert_eq!(token_client.balance(&client.address), 500);

    client.release(&escrow_id);

    assert_eq!(token_client.balance(&seller), 500);
    assert_eq!(token_client.balance(&client.address), 0);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Released);
}

#[test]
fn dispute_resolves_with_split_multisig_arbiter() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let token_admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    // 2-of-3 arbiter: all three in the config, threshold = 2.
    // mock_all_auths() satisfies every require_auth, so all three would pass.
    let (arbiter, _, _, _) = make_arbiter(&env);

    let (asset, asset_admin, token_client) = issue_token(&env, &token_admin);
    asset_admin.mint(&buyer, &1_000);

    let conditions = BytesN::from_array(&env, &[1u8; 32]);
    let timeout = env.ledger().timestamp() + 3600;
    let escrow_id = client.create(
        &buyer,
        &seller,
        &arbiter,
        &asset,
        &1_000,
        &timeout,
        &conditions,
    );

    let reason = BytesN::from_array(&env, &[2u8; 32]);
    client.dispute(&escrow_id, &buyer, &reason);

    // With mock_all_auths the M-of-N check is satisfied automatically.
    client.resolve(&escrow_id, &Decision::Split(6_000));

    assert_eq!(token_client.balance(&seller), 600);
    assert_eq!(token_client.balance(&buyer), 400);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Resolved);
}

#[test]
fn dispute_resolves_release_to_seller_multisig_arbiter() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let token_admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let (arbiter, _, _, _) = make_arbiter(&env);

    let (asset, asset_admin, token_client) = issue_token(&env, &token_admin);
    asset_admin.mint(&buyer, &1_000);

    let conditions = BytesN::from_array(&env, &[5u8; 32]);
    let timeout = env.ledger().timestamp() + 3600;
    let escrow_id = client.create(
        &buyer,
        &seller,
        &arbiter,
        &asset,
        &1_000,
        &timeout,
        &conditions,
    );

    let reason = BytesN::from_array(&env, &[6u8; 32]);
    client.dispute(&escrow_id, &seller, &reason);
    client.resolve(&escrow_id, &Decision::ReleaseToSeller);

    assert_eq!(token_client.balance(&seller), 1_000);
    assert_eq!(token_client.balance(&buyer), 0);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Resolved);
}

#[test]
fn dispute_resolves_refund_to_buyer_multisig_arbiter() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let token_admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let (arbiter, _, _, _) = make_arbiter(&env);

    let (asset, asset_admin, token_client) = issue_token(&env, &token_admin);
    asset_admin.mint(&buyer, &1_000);

    let conditions = BytesN::from_array(&env, &[7u8; 32]);
    let timeout = env.ledger().timestamp() + 3600;
    let escrow_id = client.create(
        &buyer,
        &seller,
        &arbiter,
        &asset,
        &1_000,
        &timeout,
        &conditions,
    );

    let reason = BytesN::from_array(&env, &[8u8; 32]);
    client.dispute(&escrow_id, &buyer, &reason);
    client.resolve(&escrow_id, &Decision::RefundToBuyer);

    assert_eq!(token_client.balance(&buyer), 1_000);
    assert_eq!(token_client.balance(&seller), 0);
    assert_eq!(client.get_escrow(&escrow_id).status, EscrowStatus::Resolved);
}

#[test]
fn non_party_cannot_dispute() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let token_admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);
    let (arbiter, _, _, _) = make_arbiter(&env);
    let stranger = Address::generate(&env);

    let (asset, asset_admin, _) = issue_token(&env, &token_admin);
    asset_admin.mint(&buyer, &1_000);

    let conditions = BytesN::from_array(&env, &[3u8; 32]);
    let timeout = env.ledger().timestamp() + 3600;
    let escrow_id = client.create(
        &buyer,
        &seller,
        &arbiter,
        &asset,
        &500,
        &timeout,
        &conditions,
    );

    let reason = BytesN::from_array(&env, &[4u8; 32]);
    let result = client.try_dispute(&escrow_id, &stranger, &reason);
    assert_eq!(result, Err(Ok(Error::NotParty)));
}

#[test]
fn invalid_arbiter_config_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let token_admin = Address::generate(&env);
    let buyer = Address::generate(&env);
    let seller = Address::generate(&env);

    let (asset, asset_admin, _) = issue_token(&env, &token_admin);
    asset_admin.mint(&buyer, &1_000);

    let conditions = BytesN::from_array(&env, &[9u8; 32]);
    let timeout = env.ledger().timestamp() + 3600;

    // threshold > signer count — must fail.
    let bad_arbiter = MultisigConfig {
        signers: vec![&env, Address::generate(&env)],
        threshold: 2,
    };
    let result = client.try_create(
        &buyer,
        &seller,
        &bad_arbiter,
        &asset,
        &500,
        &timeout,
        &conditions,
    );
    assert_eq!(result, Err(Ok(Error::InvalidArbiterConfig)));
}

// =============================================================================
// Timelocked admin handover — wiring tests for the escrow contract
// =============================================================================

#[test]
fn admin_handover_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let new_admin = Address::generate(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_admin(&new_admin);

    // Warp exactly to the point the timelock opens (48 hours = 172 800 s).
    env.ledger().set_timestamp(t0 + 172_800);

    let returned = client.accept_admin();
    assert_eq!(returned, new_admin);
}

#[test]
#[should_panic(expected = "timelock not elapsed")]
fn admin_handover_too_early_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let new_admin = Address::generate(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_admin(&new_admin);

    // One second before the window opens — must panic.
    env.ledger().set_timestamp(t0 + 172_800 - 1);
    client.accept_admin();
}

#[test]
#[should_panic(expected = "no pending admin change")]
fn admin_handover_no_pending_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);

    // No propose_admin was ever called.
    client.accept_admin();
}

#[test]
fn admin_handover_second_propose_overwrites_first() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, client) = setup(&env);
    let candidate_a = Address::generate(&env);
    let candidate_b = Address::generate(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_admin(&candidate_a);

    // Propose a different candidate before the first proposal is accepted.
    env.ledger().set_timestamp(t0 + 60);
    client.propose_admin(&candidate_b);

    // Warp past the second proposal's timelock (t0 + 60 + 172 800).
    env.ledger().set_timestamp(t0 + 60 + 172_800);

    let result = client.accept_admin();
    assert_eq!(result, candidate_b);
}
