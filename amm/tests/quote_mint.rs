//! Offline quote tests against the devnet `bundle-4` fixtures.
#![cfg(feature = "devnet")]

use amm::errors::NeutralBundleError;
use jupiter_amm_interface::{Amm, QuoteParams, SwapMode};

mod utils;
use utils::*;

fn params(amm: &amm::NeutralBundleAmm, amount: u64) -> QuoteParams {
    QuoteParams {
        input_mint: amm.keys.asset_mint,
        output_mint: amm.keys.mint,
        amount,
        swap_mode: SwapMode::ExactIn,
    }
}

fn error_of(result: anyhow::Result<jupiter_amm_interface::Quote>) -> NeutralBundleError {
    *result
        .unwrap_err()
        .downcast_ref::<NeutralBundleError>()
        .unwrap()
}

#[test]
fn quotes_mint_with_fees() {
    let amm = build_amm(
        &load_account_map_from_file(FIXTURE_PATHS),
        fixture_clock().unix_timestamp,
    );
    assert!(amm.is_active());
    assert!(amm.unidirectional());
    assert_eq!(
        amm.get_reserve_mints(),
        vec![amm.keys.asset_mint, amm.keys.mint]
    );

    let quote = amm.quote(&params(&amm, 1_000_000)).unwrap();
    assert_eq!(quote.in_amount, 1_000_000);
    assert!(quote.out_amount > 0);
    // bundle-4 charges a 0.5% deposit fee, taken in the asset.
    assert_eq!(quote.fee_amount, 5_000);
    assert_eq!(quote.fee_mint, amm.keys.asset_mint);

    let larger = amm.quote(&params(&amm, 2_000_000)).unwrap();
    assert!(larger.out_amount >= 2 * quote.out_amount - 1);
}

#[test]
fn later_clock_accrues_management_fee_and_mints_more() {
    let account_map = load_account_map_from_file(FIXTURE_PATHS);
    let now = fixture_clock().unix_timestamp;
    let amount = 10_000_000_000;
    let at_snapshot = build_amm(&account_map, now);
    let year_later = build_amm(&account_map, now + 365 * 86_400);
    // Management fee shares leave the pool, so each receipt token is backed
    // by fewer shares and a deposit mints more tokens.
    assert!(
        year_later
            .quote(&params(&year_later, amount))
            .unwrap()
            .out_amount
            > at_snapshot
                .quote(&params(&at_snapshot, amount))
                .unwrap()
                .out_amount
    );
}

#[test]
fn rejects_unsupported_requests() {
    let amm = build_amm(
        &load_account_map_from_file(FIXTURE_PATHS),
        fixture_clock().unix_timestamp,
    );

    let reverse = QuoteParams {
        input_mint: amm.keys.mint,
        output_mint: amm.keys.asset_mint,
        ..params(&amm, 1_000_000)
    };
    assert_eq!(
        error_of(amm.quote(&reverse)),
        NeutralBundleError::InvalidMintPair
    );

    let exact_out = QuoteParams {
        swap_mode: SwapMode::ExactOut,
        ..params(&amm, 1_000_000)
    };
    assert_eq!(
        error_of(amm.quote(&exact_out)),
        NeutralBundleError::ExactOutNotSupported
    );

    let below_minimum = amm.bundle.min_deposit_amount - 1;
    assert_eq!(
        error_of(amm.quote(&params(&amm, below_minimum))),
        NeutralBundleError::AmountTooSmall
    );
    assert_eq!(
        error_of(amm.quote(&params(&amm, amm.bundle.max_deposit_amount + 1))),
        NeutralBundleError::CapacityExceeded
    );
}
