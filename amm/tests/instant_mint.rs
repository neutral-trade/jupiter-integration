//! End-to-end: execute `instant_mint_bundle_tokens` from the AMM's account
//! metas against the devnet `ntbundle` binary and require the minted amount to
//! equal the quote exactly, refreshing the AMM between mints.
#![cfg(feature = "devnet")]

use amm::{constants::NEUTRAL_BUNDLE_PROGRAM, instant_mint_data};
use jupiter_amm_interface::{Amm, QuoteParams, SwapMode, SwapParams};
use solana_program_test::ProgramTest;
use solana_sdk::{
    instruction::Instruction,
    program_pack::Pack,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account,
};
use spl_token::state::Account as TokenAccount;

mod utils;
use utils::*;

#[tokio::test]
async fn instant_mint_matches_quote() {
    let mut program_test = ProgramTest::default();
    program_test.prefer_bpf(true);
    // Loads tests/fixtures/ntbundle.so, dumped from devnet.
    program_test.add_program("ntbundle", NEUTRAL_BUNDLE_PROGRAM, None);

    let mut account_map = load_account_map_from_file(FIXTURE_PATHS);
    let fixture_keys: Vec<Pubkey> = account_map.keys().copied().collect();
    let clock = fixture_clock();
    let amm = build_amm(&account_map, clock.unix_timestamp);
    let (asset_mint, receipt_mint) = (amm.keys.asset_mint, amm.keys.mint);

    // Take over the asset mint authority so the test can fund the user.
    let mint_authority = Keypair::new();
    account_map.get_mut(&asset_mint).unwrap().data[4..36]
        .copy_from_slice(&mint_authority.pubkey().to_bytes());
    for (address, account) in account_map.iter() {
        program_test.add_account(*address, account.clone());
    }

    let mut context = program_test.start_with_context().await;
    // The pool's fee crank reads the clock; run at the snapshot's time.
    context.set_sysvar(&clock);
    let payer = context.payer.insecure_clone();

    let user_asset = get_associated_token_address(&payer.pubkey(), &asset_mint);
    let user_receipt = get_associated_token_address(&payer.pubkey(), &receipt_mint);
    let amounts = [
        amm.bundle.min_deposit_amount,
        1_234_567,
        1_000_000_000,
        5_000_000_007,
    ];
    let setup = Transaction::new_signed_with_payer(
        &[
            create_associated_token_account(
                &payer.pubkey(),
                &payer.pubkey(),
                &asset_mint,
                &spl_token::ID,
            ),
            create_associated_token_account(
                &payer.pubkey(),
                &payer.pubkey(),
                &receipt_mint,
                &spl_token::ID,
            ),
            spl_token::instruction::mint_to(
                &spl_token::ID,
                &asset_mint,
                &user_asset,
                &mint_authority.pubkey(),
                &[],
                amounts.iter().sum(),
            )
            .unwrap(),
        ],
        Some(&payer.pubkey()),
        &[&payer, &mint_authority],
        context.last_blockhash,
    );
    context
        .banks_client
        .process_transaction(setup)
        .await
        .unwrap();

    for amount in amounts {
        // Re-quote from post-mint chain state each round.
        let account_map =
            load_account_map_from_bank(&mut context.banks_client, &fixture_keys).await;
        let amm = build_amm(&account_map, clock.unix_timestamp);
        let quote = amm
            .quote(&QuoteParams {
                input_mint: asset_mint,
                output_mint: receipt_mint,
                amount,
                swap_mode: SwapMode::ExactIn,
            })
            .unwrap();

        let jupiter_program_id = Pubkey::new_unique();
        let mut swap = amm
            .get_swap_and_account_metas(&SwapParams {
                swap_mode: SwapMode::ExactIn,
                in_amount: amount,
                out_amount: quote.out_amount,
                source_mint: asset_mint,
                destination_mint: receipt_mint,
                source_token_account: user_asset,
                destination_token_account: user_receipt,
                token_transfer_authority: payer.pubkey(),
                open_order_address: None,
                quote_mint_to_referrer: None,
                jupiter_program_id: &jupiter_program_id,
                missing_dynamic_accounts_as_default: false,
            })
            .unwrap();
        // Drop Jupiter's trailing placeholder; the rest is the program's layout.
        assert_eq!(swap.account_metas.pop().unwrap().pubkey, jupiter_program_id);

        let before = receipt_balance(&mut context.banks_client, user_receipt).await;
        let blockhash = context.get_new_latest_blockhash().await.unwrap();
        let mint_tx = Transaction::new_signed_with_payer(
            &[Instruction {
                program_id: NEUTRAL_BUNDLE_PROGRAM,
                accounts: swap.account_metas,
                // Requiring exactly the quoted amount makes the program enforce it too.
                data: instant_mint_data(amount, quote.out_amount),
            }],
            Some(&payer.pubkey()),
            &[&payer],
            blockhash,
        );
        context
            .banks_client
            .process_transaction(mint_tx)
            .await
            .unwrap();

        let minted = receipt_balance(&mut context.banks_client, user_receipt).await - before;
        println!("amount={amount} quote={} minted={minted}", quote.out_amount);
        assert_eq!(
            minted, quote.out_amount,
            "minted != quote at amount {amount}"
        );
    }
}

async fn receipt_balance(
    banks_client: &mut solana_program_test::BanksClient,
    account: Pubkey,
) -> u64 {
    let account = banks_client.get_account(account).await.unwrap().unwrap();
    TokenAccount::unpack(&account.data).unwrap().amount
}
