#![cfg(test)]

use super::*;
use soroban_sdk::testutils::Address as _;

fn setup(env: &Env) -> (Address, Address, RegistryAnchorContractClient<'_>) {
    let admin = Address::generate(env);
    let oracle = Address::generate(env);
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
    assert_eq!(flag.anchored_by, oracle);

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
    let new_oracle = Address::generate(&env);

    let t0: u64 = 1_000_000;
    env.ledger().set_timestamp(t0);

    client.propose_oracle(&new_oracle);

    env.ledger().set_timestamp(t0 + 172_800);

    let returned = client.accept_oracle();
    assert_eq!(returned, new_oracle);
}

#[test]
#[should_panic(expected = "timelock not elapsed")]
fn oracle_handover_too_early_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let (_, _, client) = setup(&env);
    let new_oracle = Address::generate(&env);

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
