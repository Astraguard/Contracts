#![cfg(test)]

use super::*;
use astraguard_shared::access::MultisigConfig;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::vec;

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

fn setup(env: &Env) -> (Address, MultisigConfig, RegistryAnchorContractClient<'_>) {
    let admin = Address::generate(env);
    let (oracle, _, _, _) = make_oracle(env);
    let contract_id = env.register(RegistryAnchorContract, ());
    let client = RegistryAnchorContractClient::new(env, &contract_id);
    client.initialize(&admin, &oracle);
    (admin, oracle, client)
}

#[test]
fn anchor_and_query_flag() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, oracle, client) = setup(&env);
    let subject = Address::generate(&env);
    let record_hash = BytesN::from_array(&env, &[9u8; 32]);

    let flag_id = client.anchor_flag(&subject, &record_hash, &FlagCategory::RugPull);

    let flag = client.get_flag(&flag_id);
    assert_eq!(flag.subject, subject);
    assert_eq!(flag.category, FlagCategory::RugPull);
    // anchored_by should match the oracle multisig config (same threshold).
    assert_eq!(flag.anchored_by.threshold, oracle.threshold);
    assert_eq!(flag.anchored_by.signers.len(), oracle.signers.len());

    let subject_flags = client.get_flags_for_subject(&subject);
    assert_eq!(subject_flags.len(), 1);
    assert_eq!(subject_flags.get(0).unwrap(), flag_id);
}

#[test]
fn multiple_flags_accumulate_per_subject() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, client) = setup(&env);
    let subject = Address::generate(&env);

    client.anchor_flag(
        &subject,
        &BytesN::from_array(&env, &[1u8; 32]),
        &FlagCategory::Scam,
    );
    client.anchor_flag(
        &subject,
        &BytesN::from_array(&env, &[2u8; 32]),
        &FlagCategory::Phishing,
    );

    assert_eq!(client.get_flags_for_subject(&subject).len(), 2);
}

#[test]
fn supersede_flag_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, client) = setup(&env);
    let subject = Address::generate(&env);

    let flag_id = client.anchor_flag(
        &subject,
        &BytesN::from_array(&env, &[3u8; 32]),
        &FlagCategory::FakeTeam,
    );

    let reason = BytesN::from_array(&env, &[4u8; 32]);
    let supersession = client.supersede_flag(&flag_id, &reason);
    assert_eq!(supersession.reason_hash, reason);
    assert_eq!(supersession.superseded_by.threshold, 2);
}

#[test]
fn supersede_flag_idempotency_guard() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, client) = setup(&env);
    let subject = Address::generate(&env);

    let flag_id = client.anchor_flag(
        &subject,
        &BytesN::from_array(&env, &[5u8; 32]),
        &FlagCategory::Other,
    );

    client.supersede_flag(&flag_id, &BytesN::from_array(&env, &[6u8; 32]));

    // Second call must fail.
    let result = client.try_supersede_flag(&flag_id, &BytesN::from_array(&env, &[7u8; 32]));
    assert_eq!(result, Err(Ok(Error::AlreadySuperseded)));
}

#[test]
fn supersede_flag_unknown_flag_guard() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, client) = setup(&env);

    let result = client.try_supersede_flag(&999u64, &BytesN::from_array(&env, &[8u8; 32]));
    assert_eq!(result, Err(Ok(Error::FlagNotFound)));
}

#[test]
fn get_flag_with_supersession_live_vs_retracted() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, client) = setup(&env);
    let subject = Address::generate(&env);

    let flag_id = client.anchor_flag(
        &subject,
        &BytesN::from_array(&env, &[10u8; 32]),
        &FlagCategory::Scam,
    );

    // Before supersession — should be live (supersession = None).
    let live = client.get_flag_with_supersession(&flag_id);
    assert!(live.supersession.is_none());

    // After supersession — should be retracted.
    client.supersede_flag(&flag_id, &BytesN::from_array(&env, &[11u8; 32]));
    let retracted = client.get_flag_with_supersession(&flag_id);
    assert!(retracted.supersession.is_some());
}

#[test]
fn invalid_oracle_config_rejected_on_initialize() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);

    // threshold = 0 — must fail.
    let bad_oracle = MultisigConfig {
        signers: vec![&env, Address::generate(&env)],
        threshold: 0,
    };

    let contract_id = env.register(RegistryAnchorContract, ());
    let client = RegistryAnchorContractClient::new(&env, &contract_id);
    let result = client.try_initialize(&admin, &bad_oracle);
    assert_eq!(result, Err(Ok(Error::InvalidOracleConfig)));
}

// =============================================================================
// Timelocked admin and oracle handover — wiring tests for the registry-anchor
// =============================================================================

#[test]
fn admin_handover_happy_path() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, client) = setup(&env);
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

    let (_, _, client) = setup(&env);
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

    let (_, _, client) = setup(&env);
    client.accept_admin();
}

#[test]
fn admin_handover_second_propose_overwrites_first() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, client) = setup(&env);
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

    let (_, _, client) = setup(&env);
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

    let (_, _, client) = setup(&env);
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

    let (_, _, client) = setup(&env);
    client.accept_oracle();
}
