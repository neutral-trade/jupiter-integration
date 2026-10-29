use solana_sdk::{pubkey, pubkey::Pubkey};

/// `ntbundle` mainnet deployment (default, non-`jupiter` build).
#[cfg(not(feature = "devnet"))]
pub const NEUTRAL_BUNDLE_PROGRAM: Pubkey = pubkey!("BUNDDh4P5XviMm1f3gCvnq2qKx6TGosAGnoUK12e7cXU");
/// `ntbundle` devnet deployment; same account layout as mainnet.
#[cfg(feature = "devnet")]
pub const NEUTRAL_BUNDLE_PROGRAM: Pubkey = pubkey!("CcR9whVnaW3STx6LLYotwy5JJXmnC8KDjtRotA3NCL8v");

pub const TOKEN_PROGRAM: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const ATA_PROGRAM: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub const BUNDLE_ASSET_AUTHORITY_SEED: &[u8] = b"BUNDLE_ASSET_AUTHORITY";
pub const ORACLE_SEED: &[u8] = b"ORACLE";
pub const BUNDLE_TEMP_DATA_SEED: &[u8] = b"BUNDLE_TEMP_DATA";
pub const TOKENIZED_BUNDLE_DEPOSITOR_SEED: &[u8] = b"TOKENIZED_BUNDLE_DEPOSITOR";
pub const TOKENIZED_BUNDLE_MINT_SEED: &[u8] = b"TOKENIZED_BUNDLE_MINT";
pub const TOKENIZED_BUNDLE_TOKEN_AUTHORITY_SEED: &[u8] = b"TOKENIZED_BUNDLE_TOKEN_AUTHORITY";

/// `sha256("global:instant_mint_bundle_tokens")[..8]`
pub const INSTANT_MINT_BUNDLE_TOKENS_DISCRIMINATOR: [u8; 8] = [56, 192, 88, 145, 75, 250, 148, 33];

pub const AMM_LABEL: &str = "NeutralBundle";
