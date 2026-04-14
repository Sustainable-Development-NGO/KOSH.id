use anchor_lang::prelude::*;

// ─────────────────────────────────────────────────────────
//  KOSH.id v1 — account types and shared constants
// ─────────────────────────────────────────────────────────

// ── Constants ──────────────────────────────────────────────────────────────

/// Approximate Solana slot rate: ~2 slots/sec × 31,536,000 sec/year
pub const SLOTS_PER_YEAR: u64 = 78_840_000;

/// Reputation range [0, 10_000]
pub const MAX_REPUTATION: u64 = 10_000;

/// Minimum oracle-signed payload length (arbitrary lower bound).
pub const MIN_ORACLE_PAYLOAD_LEN: usize = 1;

// ── Account structs ────────────────────────────────────────────────────────

/// Protocol-wide configuration.
/// Seeds: [b"config"]
#[account]
pub struct GlobalConfig {
    /// The authority key is also the trusted oracle signer for
    /// `update_reputation` payloads.
    pub authority: Pubkey,
    /// Fee taken on loan origination, in basis points.
    pub protocol_fee_bps: u16,
    /// Base per-slot interest rate, in basis points.
    pub base_interest_bps: u16,
    /// When true, all state-changing instructions are halted.
    pub emergency_pause: bool,
    /// Minimum reputation_score required to take a loan.
    pub min_reputation_for_loan: u64,
    /// 1-hour TWAP change threshold; trips circuit breaker.
    /// 1000 = 10%.
    pub twap_freeze_threshold_bps: u16,
    /// Re-entrancy lock — set to `true` at the start of any CPI-
    /// performing instruction and cleared on exit.
    ///
    /// Solana's single-threaded execution means true concurrent re-
    /// entrancy (like EVM) cannot happen.  However a malicious program
    /// invoked via CPI could call back into this program before our
    /// own CPI returns.  Checking this flag on entry of every CPI-
    /// performing instruction prevents that cross-program callback path.
    pub reentrancy_lock: bool,
}

impl GlobalConfig {
    pub const LEN: usize = 8   // discriminator
        + 32   // authority
        + 2    // protocol_fee_bps
        + 2    // base_interest_bps
        + 1    // emergency_pause
        + 8    // min_reputation_for_loan
        + 2    // twap_freeze_threshold_bps
        + 1;   // reentrancy_lock
}

/// Per-user reputation profile.
/// Seeds: [b"kosh_profile", user.key()]
#[account]
pub struct UserReputationProfile {
    pub user: Pubkey,
    /// Score in [0, 10_000].
    pub reputation_score: u64,
    /// Slot at which the score was last updated.
    pub last_update_slot: u64,
    /// SHA-256 of the most-recently accepted oracle payload.
    /// Acts as a nullifier: reject any payload whose hash equals
    /// this value to prevent double-spend of oracle attestations.
    pub oracle_nullifier: [u8; 32],
    /// Number of currently open loans (v1 cap = 1).
    pub active_loan_count: u8,
}

impl UserReputationProfile {
    pub const LEN: usize = 8   // discriminator
        + 32   // user
        + 8    // reputation_score
        + 8    // last_update_slot
        + 32   // oracle_nullifier
        + 1;   // active_loan_count
}

/// Per-loan state.
/// Seeds: [b"loan", user.key(), loan_id.to_le_bytes()]
#[account]
pub struct LoanAccount {
    pub user: Pubkey,
    pub loan_id: u64,
    pub principal_lamports: u64,
    pub interest_rate_bps: u16,
    pub start_slot: u64,
    pub status: LoanStatus,
}

impl LoanAccount {
    pub const LEN: usize = 8   // discriminator
        + 32   // user
        + 8    // loan_id
        + 8    // principal_lamports
        + 2    // interest_rate_bps
        + 8    // start_slot
        + 1;   // status (enum tag)
}

/// Lifecycle of a single loan.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum LoanStatus {
    Active,
    Repaid,
    Defaulted,
}
