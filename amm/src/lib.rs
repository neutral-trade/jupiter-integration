//! Jupiter AMM integration for Neutral Trade bundles (`ntbundle`).
//!
//! A bundle is a managed vault whose tokenized share pool issues an SPL receipt
//! token. The only atomic, permissionless swap the program exposes is
//! `instant_mint_bundle_tokens`: deposit the bundle asset, receive receipt
//! tokens in the same instruction. Redemption (`request_token_withdrawal`) is
//! queued for keeper settlement in a later netting cycle, so it cannot be
//! routed and this AMM is unidirectional: asset -> receipt token.
//!
//! `quote` replays the on-chain math exactly: deposit fee, bundle and cycle
//! caps, the pool's management/performance fee crank at the current clock, the
//! issuance fee, the share conversion, and the virtual-offset token conversion.

use std::sync::atomic::Ordering;

use anyhow::Result;
use jupiter_amm_interface::{
    try_get_account_data, AccountMap, Amm, AmmContext, ClockRef, KeyedAccount, Quote, QuoteParams,
    Swap, SwapAndAccountMetas, SwapMode, SwapParams,
};
use rust_decimal::Decimal;
use solana_sdk::instruction::AccountMeta;
use solana_sdk::pubkey::Pubkey;

pub mod constants;
pub mod errors;
pub mod math;
pub mod state;

use constants::*;
use errors::NeutralBundleError;
use math::{fee_amount_ceil, pool_shares_after_fees, MintCurve, MintRejection};
use state::{load_oracle_external_equity, Bundle, BundleTempData, TokenizedBundleDepositor};

fn bundle_pda(seed: &[u8], bundle: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seed, bundle.as_ref()], &NEUTRAL_BUNDLE_PROGRAM).0
}

/// SPL Token associated token account of `owner` for `mint`.
pub fn get_associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM.as_ref(), mint.as_ref()],
        &ATA_PROGRAM,
    )
    .0
}

/// The receipt-token mint of `bundle`.
pub fn derive_bundle_mint(bundle: &Pubkey) -> Pubkey {
    bundle_pda(TOKENIZED_BUNDLE_MINT_SEED, bundle)
}

/// `(supply, decimals)` of an initialized SPL Token mint.
fn load_spl_mint(owner: &Pubkey, data: &[u8]) -> Result<(u64, u8), NeutralBundleError> {
    if *owner != TOKEN_PROGRAM {
        return Err(NeutralBundleError::UnsupportedMint);
    }
    if data.len() != 82 || data[45] != 1 {
        return Err(NeutralBundleError::Deserialization("invalid SPL mint"));
    }
    let supply = u64::from_le_bytes(data[36..44].try_into().unwrap());
    Ok((supply, data[44]))
}

/// Addresses derived from the bundle key and its state.
#[derive(Clone, Copy, Debug)]
pub struct BundleKeys {
    pub bundle: Pubkey,
    pub temp_data: Pubkey,
    pub oracle: Pubkey,
    pub tokenized_depositor: Pubkey,
    pub mint: Pubkey,
    pub token_authority: Pubkey,
    pub asset_authority: Pubkey,
    pub asset_mint: Pubkey,
    /// The bundle's asset vault: the asset ATA of `asset_authority`.
    pub asset_vault: Pubkey,
    pub treasury: Pubkey,
}

impl BundleKeys {
    pub fn derive(bundle_key: Pubkey, bundle: &Bundle) -> Self {
        let asset_authority = bundle_pda(BUNDLE_ASSET_AUTHORITY_SEED, &bundle_key);
        Self {
            bundle: bundle_key,
            temp_data: bundle_pda(BUNDLE_TEMP_DATA_SEED, &bundle_key),
            oracle: bundle_pda(ORACLE_SEED, &bundle_key),
            tokenized_depositor: bundle_pda(TOKENIZED_BUNDLE_DEPOSITOR_SEED, &bundle_key),
            mint: derive_bundle_mint(&bundle_key),
            token_authority: bundle_pda(TOKENIZED_BUNDLE_TOKEN_AUTHORITY_SEED, &bundle_key),
            asset_authority,
            asset_mint: bundle.asset_address,
            asset_vault: get_associated_token_address(&asset_authority, &bundle.asset_address),
            treasury: bundle.treasury_account,
        }
    }
}

/// Account metas for `instant_mint_bundle_tokens`, in `InstantMintBundleTokens`
/// order. `user` must own both token accounts and sign.
#[derive(Copy, Clone, Debug)]
pub struct NeutralBundleSwap {
    pub keys: BundleKeys,
    pub user: Pubkey,
    pub user_asset_account: Pubkey,
    pub user_receipt_account: Pubkey,
}

impl From<NeutralBundleSwap> for Vec<AccountMeta> {
    fn from(swap: NeutralBundleSwap) -> Self {
        let keys = swap.keys;
        vec![
            // The program declares `user` as `#[account(mut)]`.
            AccountMeta::new(swap.user, true),
            AccountMeta::new(swap.user_asset_account, false),
            AccountMeta::new(keys.asset_vault, false),
            AccountMeta::new_readonly(keys.asset_authority, false),
            AccountMeta::new(keys.treasury, false),
            AccountMeta::new(keys.bundle, false),
            AccountMeta::new_readonly(keys.temp_data, false),
            AccountMeta::new_readonly(keys.oracle, false),
            AccountMeta::new(keys.tokenized_depositor, false),
            AccountMeta::new_readonly(keys.asset_mint, false),
            AccountMeta::new(keys.mint, false),
            AccountMeta::new_readonly(keys.token_authority, false),
            AccountMeta::new(swap.user_receipt_account, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM, false),
        ]
    }
}

/// Instruction data for `instant_mint_bundle_tokens(amount_in, min_tokens_out)`.
pub fn instant_mint_data(amount_in: u64, min_tokens_out: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&INSTANT_MINT_BUNDLE_TOKENS_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_tokens_out.to_le_bytes());
    data
}

#[derive(Clone)]
pub struct NeutralBundleAmm {
    pub keys: BundleKeys,
    pub bundle: Bundle,
    pub temp_data: BundleTempData,
    pub pool: TokenizedBundleDepositor,
    pub external_equity: u64,
    pub mint_supply: u64,
    clock_ref: ClockRef,
    /// Why the bundle cannot mint right now (paused, distributing, ...).
    halted: Option<&'static str>,
    updated: bool,
}

impl NeutralBundleAmm {
    pub fn new(bundle_key: Pubkey, bundle: Bundle, clock_ref: ClockRef) -> Self {
        Self {
            keys: BundleKeys::derive(bundle_key, &bundle),
            bundle,
            temp_data: BundleTempData::default(),
            pool: TokenizedBundleDepositor::default(),
            external_equity: 0,
            mint_supply: 0,
            clock_ref,
            halted: None,
            updated: false,
        }
    }

    /// The mint curve with the pool's fee crank evaluated at `current_time`,
    /// as `instant_mint_bundle_tokens` would run it.
    pub fn mint_curve(&self, current_time: i64) -> Result<MintCurve, NeutralBundleError> {
        let total_assets = self
            .bundle
            .bundle_underlying_balance
            .checked_add(self.external_equity)
            .ok_or(NeutralBundleError::MathOverflow)?;
        let pool_shares = pool_shares_after_fees(
            &self.bundle,
            &self.pool,
            total_assets,
            self.mint_supply,
            current_time,
        )?;
        if self.mint_supply != 0 && pool_shares == 0 {
            return Err(NeutralBundleError::Halted(
                "tokenized bundle depositor state invalid",
            ));
        }
        let cycle_minted =
            if self.pool.current_cycle_netting_timestamp == self.temp_data.last_netting_timestamp {
                self.pool.current_cycle_minted
            } else {
                0
            };

        Ok(MintCurve {
            deposit_fee_bps: self.bundle.deposit_fee,
            issuance_fee_bps: self.bundle.issuance_fee_bps,
            min_deposit_amount: self.bundle.min_deposit_amount,
            total_assets,
            total_shares: self.bundle.total_shares,
            committed_assets: total_assets
                .checked_add(self.temp_data.cumulative_pending_deposits)
                .ok_or(NeutralBundleError::MathOverflow)?,
            max_deposit_amount: self.bundle.max_deposit_amount,
            cycle_minted,
            mint_cap_per_cycle: self.pool.mint_cap_per_cycle,
            token_supply: self.mint_supply,
            pool_shares,
        })
    }
}

impl Amm for NeutralBundleAmm {
    fn from_keyed_account(keyed_account: &KeyedAccount, amm_context: &AmmContext) -> Result<Self> {
        if keyed_account.account.owner != NEUTRAL_BUNDLE_PROGRAM {
            return Err(NeutralBundleError::Deserialization("bundle not owned by ntbundle").into());
        }
        let bundle = Bundle::load(&keyed_account.account.data)?;
        Ok(Self::new(
            keyed_account.key,
            bundle,
            amm_context.clock_ref.clone(),
        ))
    }

    fn label(&self) -> String {
        AMM_LABEL.to_owned()
    }

    fn program_id(&self) -> Pubkey {
        NEUTRAL_BUNDLE_PROGRAM
    }

    fn key(&self) -> Pubkey {
        self.keys.bundle
    }

    fn get_reserve_mints(&self) -> Vec<Pubkey> {
        vec![self.keys.asset_mint, self.keys.mint]
    }

    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        vec![
            self.keys.bundle,
            self.keys.temp_data,
            self.keys.oracle,
            self.keys.tokenized_depositor,
            self.keys.mint,
            self.keys.asset_mint,
        ]
    }

    fn update(&mut self, account_map: &AccountMap) -> Result<()> {
        let bundle = Bundle::load(try_get_account_data(account_map, &self.keys.bundle)?)?;
        let temp_data =
            BundleTempData::load(try_get_account_data(account_map, &self.keys.temp_data)?)?;
        let external_equity =
            load_oracle_external_equity(try_get_account_data(account_map, &self.keys.oracle)?)?;
        let pool = TokenizedBundleDepositor::load(try_get_account_data(
            account_map,
            &self.keys.tokenized_depositor,
        )?)?;
        if pool.bundle != self.keys.bundle || pool.mint != self.keys.mint {
            return Err(NeutralBundleError::Deserialization(
                "tokenized depositor does not belong to this bundle",
            )
            .into());
        }

        let account = |key: &Pubkey| {
            account_map
                .get(key)
                .ok_or_else(|| anyhow::anyhow!("Could not find address: {key}"))
        };
        let mint = account(&self.keys.mint)?;
        let (mint_supply, mint_decimals) = load_spl_mint(&mint.owner, &mint.data)?;
        let asset_mint = account(&bundle.asset_address)?;
        let (_, asset_decimals) = load_spl_mint(&asset_mint.owner, &asset_mint.data)?;

        self.halted = if temp_data.paused_deposits_withdrawals {
            Some("bundle deposits and withdrawals are paused")
        } else if temp_data.is_distribution_started {
            Some("bundle distribution in progress")
        } else if bundle.permissionned {
            Some("permissioned bundles cannot be tokenized")
        } else if asset_decimals != bundle.asset_decimals || mint_decimals != bundle.asset_decimals
        {
            Some("bundle asset precision mismatch")
        } else {
            None
        };

        self.keys = BundleKeys::derive(self.keys.bundle, &bundle);
        self.bundle = bundle;
        self.temp_data = temp_data;
        self.pool = pool;
        self.external_equity = external_equity;
        self.mint_supply = mint_supply;
        self.updated = true;
        Ok(())
    }

    fn quote(&self, quote_params: &QuoteParams) -> Result<Quote> {
        if quote_params.swap_mode != SwapMode::ExactIn {
            return Err(NeutralBundleError::ExactOutNotSupported.into());
        }
        if quote_params.input_mint != self.keys.asset_mint
            || quote_params.output_mint != self.keys.mint
        {
            return Err(NeutralBundleError::InvalidMintPair.into());
        }
        if let Some(reason) = self.halted {
            return Err(NeutralBundleError::Halted(reason).into());
        }

        let current_time = self.clock_ref.unix_timestamp.load(Ordering::Relaxed);
        let curve = self.mint_curve(current_time)?;
        let amount = quote_params.amount;
        let out_amount = match curve.tokens_out(amount)? {
            Ok(tokens) => tokens,
            Err(MintRejection::TooSmall) => return Err(NeutralBundleError::AmountTooSmall.into()),
            Err(MintRejection::CapacityExceeded) => {
                return Err(NeutralBundleError::CapacityExceeded.into())
            }
        };

        // Deposit and issuance fees are both taken in the asset.
        let deposit_fee = fee_amount_ceil(amount, curve.deposit_fee_bps)?;
        let issuance_fee = fee_amount_ceil(amount - deposit_fee, curve.issuance_fee_bps)?;
        let fee_amount = deposit_fee + issuance_fee;
        let fee_pct = if amount == 0 {
            Decimal::ZERO
        } else {
            Decimal::from(fee_amount) / Decimal::from(amount)
        };

        Ok(Quote {
            in_amount: amount,
            out_amount,
            fee_amount,
            fee_mint: quote_params.input_mint,
            fee_pct,
            ..Quote::default()
        })
    }

    fn get_swap_and_account_metas(&self, swap_params: &SwapParams) -> Result<SwapAndAccountMetas> {
        if swap_params.source_mint != self.keys.asset_mint
            || swap_params.destination_mint != self.keys.mint
        {
            return Err(NeutralBundleError::InvalidMintPair.into());
        }
        let mut account_metas: Vec<AccountMeta> = NeutralBundleSwap {
            keys: self.keys,
            user: swap_params.token_transfer_authority,
            user_asset_account: swap_params.source_token_account,
            user_receipt_account: swap_params.destination_token_account,
        }
        .into();
        account_metas.push(swap_params.placeholder_account_meta());

        Ok(SwapAndAccountMetas {
            // Placeholder until Jupiter assigns an `ntbundle` swap variant that
            // encodes `instant_mint_bundle_tokens(in_amount, 0)`.
            swap: Swap::TokenSwap,
            account_metas,
        })
    }

    fn clone_amm(&self) -> Box<dyn Amm + Send + Sync> {
        Box::new(self.clone())
    }

    fn unidirectional(&self) -> bool {
        true
    }

    fn get_accounts_len(&self) -> usize {
        15
    }

    fn is_active(&self) -> bool {
        self.updated && self.halted.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_discriminator_matches_anchor() {
        let hash = solana_sdk::hash::hash(b"global:instant_mint_bundle_tokens");
        assert_eq!(
            INSTANT_MINT_BUNDLE_TOKENS_DISCRIMINATOR,
            hash.to_bytes()[..8]
        );
    }
}
