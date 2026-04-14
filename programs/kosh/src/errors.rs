use anchor_lang::prelude::*;

#[error_code]
pub enum KoshError {
    /// update_reputation called before 10,000-slot cooldown has elapsed.
    #[msg("Reputation update cooldown has not elapsed (10,000 slots required)")]
    ReputationCooldown,

    /// Oracle payload hash matches the stored nullifier — already processed.
    #[msg("Duplicate oracle proof: this attestation has already been applied")]
    DuplicateProof,

    /// emergency_pause is set on GlobalConfig.
    #[msg("Protocol is paused by authority")]
    ProtocolPaused,

    /// Vault does not hold enough lamports for the requested loan.
    #[msg("Vault has insufficient lamports for this loan")]
    InsufficientVault,

    /// User already has an active loan (v1 one-loan-per-user policy).
    #[msg("User already has an active loan; repay before requesting another")]
    OverLeveraged,

    /// Pyth 1-hour TWAP moved beyond the configured threshold.
    #[msg("SOL/USD price circuit breaker tripped — lending paused")]
    CircuitBreakerTripped,

    /// Signer is not the authority registered in GlobalConfig.
    #[msg("Caller is not the authorised oracle signer")]
    UnauthorizedOracle,

    /// Re-entrancy lock is set — a CPI is already in progress.
    #[msg("Re-entrancy guard: a CPI is already in flight")]
    ReentrancyGuard,
}
