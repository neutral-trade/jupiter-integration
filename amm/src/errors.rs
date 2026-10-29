use thiserror::Error;

#[derive(Error, Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeutralBundleError {
    #[error("Invalid mint pair: only bundle asset -> receipt token is supported")]
    InvalidMintPair,

    #[error("ExactOut is not supported")]
    ExactOutNotSupported,

    #[error("Math Overflow")]
    MathOverflow,

    #[error("Account deserialization failed: {0}")]
    Deserialization(&'static str),

    #[error("Unsupported mint: instant mint requires SPL Token mints")]
    UnsupportedMint,

    #[error("Bundle cannot mint: {0}")]
    Halted(&'static str),

    #[error("Amount below the bundle minimum or rounds to zero tokens")]
    AmountTooSmall,

    #[error("Amount exceeds the bundle deposit cap or the per-cycle mint cap")]
    CapacityExceeded,
}
