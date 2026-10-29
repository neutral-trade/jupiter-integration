#![allow(dead_code)] // each test binary uses a subset of these helpers.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::Ordering;

use amm::NeutralBundleAmm;
use base64::{engine::general_purpose::STANDARD, Engine};
use jupiter_amm_interface::{AccountMap, Amm, AmmContext, ClockRef, KeyedAccount};
use serde_json::Value;
use solana_program_test::BanksClient;
use solana_sdk::{account::Account, clock::Clock, pubkey::Pubkey};

/// Devnet `bundle-4` and every account `instant_mint_bundle_tokens` touches,
/// snapshotted at one slot together with the clock sysvar.
pub const FIXTURE_PATHS: &[&str] = &[
    "tests/fixtures/bundle.json",
    "tests/fixtures/bundle_temp_data.json",
    "tests/fixtures/oracle.json",
    "tests/fixtures/tokenized_depositor.json",
    "tests/fixtures/bundle_mint.json",
    "tests/fixtures/asset_mint.json",
    "tests/fixtures/asset_vault.json",
    "tests/fixtures/treasury.json",
];

pub fn load_account_from_file(path: &str) -> (Pubkey, Account) {
    let json: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let account = &json["account"];
    (
        Pubkey::from_str(json["pubkey"].as_str().unwrap()).unwrap(),
        Account {
            lamports: account["lamports"].as_u64().unwrap(),
            data: STANDARD
                .decode(account["data"][0].as_str().unwrap())
                .unwrap(),
            owner: Pubkey::from_str(account["owner"].as_str().unwrap()).unwrap(),
            executable: account["executable"].as_bool().unwrap(),
            rent_epoch: account["rentEpoch"].as_u64().unwrap(),
        },
    )
}

pub fn load_account_map_from_file(paths: &[&str]) -> AccountMap {
    let mut map: AccountMap = HashMap::with_hasher(ahash::RandomState::new());
    for path in paths {
        let (address, account) = load_account_from_file(path);
        map.insert(address, account);
    }
    map
}

pub async fn load_account_map_from_bank(
    banks_client: &mut BanksClient,
    accounts: &[Pubkey],
) -> AccountMap {
    let mut map: AccountMap = HashMap::with_hasher(ahash::RandomState::new());
    for address in accounts {
        let account = banks_client.get_account(*address).await.unwrap().unwrap();
        map.insert(*address, account);
    }
    map
}

/// The clock sysvar captured with the fixtures.
pub fn fixture_clock() -> Clock {
    let (_, account) = load_account_from_file("tests/fixtures/clock.json");
    bincode::deserialize(&account.data).unwrap()
}

/// Build the AMM from the bundle fixture with its clock at `unix_timestamp`,
/// then update it from `account_map`.
pub fn build_amm(account_map: &AccountMap, unix_timestamp: i64) -> NeutralBundleAmm {
    let (key, account) = load_account_from_file("tests/fixtures/bundle.json");
    let clock_ref = ClockRef::default();
    clock_ref
        .unix_timestamp
        .store(unix_timestamp, Ordering::Relaxed);
    let mut amm = NeutralBundleAmm::from_keyed_account(
        &KeyedAccount {
            key,
            account,
            params: None,
        },
        &AmmContext { clock_ref },
    )
    .unwrap();
    amm.update(account_map).unwrap();
    amm
}
