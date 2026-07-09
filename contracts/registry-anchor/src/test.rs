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
