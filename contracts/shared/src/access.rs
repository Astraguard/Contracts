use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
#[derive(Clone)]
enum AccessKey {
    Admin,
    Oracle,
}

pub fn set_admin(env: &Env, admin: &Address) {
    env.storage().instance().set(&AccessKey::Admin, admin);
}

pub fn get_admin(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&AccessKey::Admin)
        .expect("admin not set")
}

pub fn has_admin(env: &Env) -> bool {
    env.storage().instance().has(&AccessKey::Admin)
}

pub fn require_admin(env: &Env) {
    get_admin(env).require_auth();
}

pub fn set_oracle(env: &Env, oracle: &Address) {
    env.storage().instance().set(&AccessKey::Oracle, oracle);
}

pub fn get_oracle(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&AccessKey::Oracle)
        .expect("oracle not set")
}

pub fn require_oracle(env: &Env) {
    get_oracle(env).require_auth();
}
