# Neutral Bundle — Jupiter AMM integration

[`jupiter-amm-interface`](https://crates.io/crates/jupiter-amm-interface) implementation for Neutral Trade bundles (`ntbundle`, mainnet `BUNDDh4P5XviMm1f3gCvnq2qKx6TGosAGnoUK12e7cXU`).

A bundle is a managed vault whose tokenized share pool issues an SPL receipt token. The program's only atomic, permissionless swap is `instant_mint_bundle_tokens`: deposit the bundle asset, receive receipt tokens. Redemption (`request_token_withdrawal`) is queued for keeper settlement, so `NeutralBundleAmm` is **unidirectional**: asset → receipt token.

## Quote

`quote` replays the on-chain path exactly:

1. deposit fee (rounded up) and the bundle minimum;
2. bundle deposit cap (including pending deposits) and the per-cycle instant-mint cap;
3. the share-token pool's management and performance fee crank at `AmmContext::clock_ref`;
4. issuance fee (rounded up), share conversion, and the virtual-offset token conversion.

Sizes the program would reject return an error (`AmountTooSmall`, `CapacityExceeded`); a paused, distributing or permissioned bundle reports `is_active() == false`. `fee_amount` is the deposit plus issuance fee, in the asset.

## Accounts

`get_accounts_to_update`: bundle, temp data, oracle, tokenized depositor, receipt mint, asset mint (all PDAs of the bundle except the asset mint).

`get_swap_and_account_metas` returns the `InstantMintBundleTokens` accounts (14) followed by Jupiter's placeholder. Notes for the Jupiter program:

- Instruction data: `sha256("global:instant_mint_bundle_tokens")[..8] ++ amount_in: u64 ++ min_tokens_out: u64` (see `instant_mint_data`). `Swap::TokenSwap` is a placeholder until an `ntbundle` swap variant exists.
- The program declares `user` as `#[account(mut)]`, so the transfer authority is passed **writable** and signing; it must own both token accounts.
- Asset and receipt mints must be SPL Token (not Token-2022).

## Tests

No bundle on mainnet has a tokenized depositor yet, so the fixtures are devnet `bundle-4` (`8TqrZmyiWQ3F3WysiBaBQC1Nzj1LDn5AR7JjXBYzFxxQ`: 10% performance, 2% management and 0.5% deposit fees), snapshotted at one slot with its clock, plus the devnet program binary. The `devnet` feature switches the program id to `CcR9whVnaW3STx6LLYotwy5JJXmnC8KDjtRotA3NCL8v`; account layouts are identical.

```bash
cd amm
cargo test                    # unit tests (mainnet program id)
cargo test --features devnet  # + fixture quotes and an end-to-end instant mint
```

`instant_mint` executes the AMM's account metas against the devnet binary with `solana-program-test` and requires the minted amount to equal the quote exactly across several sizes, re-quoting from post-mint state each round.
