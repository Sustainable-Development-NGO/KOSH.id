// ─────────────────────────────────────────────────────────────────────────
//  KOSH.id v1 — micro-lending protocol for Indian gig workers
//  Anchor 0.30.1 · Rust 1.78 · solana-program 1.18
// ─────────────────────────────────────────────────────────────────────────
use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

pub mod errors;
pub mod instructions;
pub mod state;
pub mod utils;

use state::{GlobalConfig, LoanAccount, LoanStatus, UserReputationProfile};

// Replace with the deployed program address before mainnet launch.
// Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS is the canonical
// Anchor placeholder used for undeployed programs.
declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

// TODO v2 stubs (do not implement until v1 is production-hardened):
// - Real Groth16 ZK proof verification (needs separate verifier program)
// - Kamino/Meteora yield integration
// - ZK-compressed accounts (Light Protocol)
// - Jito bundle construction
// - Vouching/social staking
// - Soulbound Karma token

// ── Instruction accounts context structs ─────────────────────────────────
//
// These MUST live in lib.rs (crate root) so Anchor's #[program] macro
// can resolve the types and the auto-generated __client_accounts_* modules
// it produces for CPI support.  Handler logic lives in instructions/*.rs.

/// One-time protocol bootstrap parameters.
#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct InitializeConfigParams {
    pub protocol_fee_bps: u16,
    pub base_interest_bps: u16,
    pub min_reputation_for_loan: u64,
    pub twap_freeze_threshold_bps: u16,
}

// ── InitializeConfig ──────────────────────────────────────────────────────

/// Accounts for `initialize_config`.
/// Compute budget: ~10k CU
#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    /// The authority that owns the protocol; also the trusted oracle key.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Protocol-wide config singleton.
    ///
    /// We use `init` (NOT `init_if_needed`) to guarantee this account
    /// is created exactly once.  `init_if_needed` would silently skip
    /// initialisation on subsequent calls, allowing the authority to
    /// overwrite fields without a dedicated upgrade path.
    #[account(
        init,
        payer = authority,
        space = GlobalConfig::LEN,
        seeds = [b"config"],
        bump
    )]
    pub config: Account<'info, GlobalConfig>,

    pub system_program: Program<'info, System>,
}

// ── InitializeUser ────────────────────────────────────────────────────────

/// Accounts for `initialize_user`.
/// Compute budget: ~15k CU
#[derive(Accounts)]
pub struct InitializeUser<'info> {
    /// The user who is creating their own profile (pays rent, signs tx).
    #[account(mut)]
    pub user: Signer<'info>,

    /// Protocol config — checked for emergency_pause.
    #[account(seeds = [b"config"], bump)]
    pub config: Account<'info, GlobalConfig>,

    /// User reputation profile.
    ///
    /// `init` (NOT `init_if_needed`) is used so that calling this
    /// instruction a second time returns an error rather than silently
    /// resetting the user's reputation_score to zero.
    #[account(
        init,
        payer = user,
        space = UserReputationProfile::LEN,
        seeds = [b"kosh_profile", user.key().as_ref()],
        bump
    )]
    pub profile: Account<'info, UserReputationProfile>,

    pub system_program: Program<'info, System>,
}

// ── UpdateReputation ──────────────────────────────────────────────────────

/// Accounts for `update_reputation`.
/// Compute budget: up to 250k CU (sha256 + Ed25519 sig verify)
#[derive(Accounts)]
pub struct UpdateReputation<'info> {
    /// The user whose reputation is being updated.
    #[account(mut)]
    pub user: Signer<'info>,

    /// Protocol config — read authority + pause flag.
    #[account(seeds = [b"config"], bump)]
    pub config: Account<'info, GlobalConfig>,

    /// The profile to update.
    #[account(
        mut,
        seeds = [b"kosh_profile", user.key().as_ref()],
        bump,
        constraint = profile.user == user.key()
    )]
    pub profile: Account<'info, UserReputationProfile>,

    /// Instructions sysvar for Ed25519 pre-instruction verification.
    /// CHECK: We read this sysvar to validate the Ed25519 sig instruction.
    #[account(address = anchor_lang::solana_program::sysvar::instructions::ID)]
    pub instructions_sysvar: AccountInfo<'info>,
}

// ── RequestLoan ───────────────────────────────────────────────────────────

/// Accounts for `request_loan`.
/// Compute budget: ~80k CU
#[derive(Accounts)]
#[instruction(loan_id: u64)]
pub struct RequestLoan<'info> {
    /// Borrower — must own the profile.
    #[account(mut)]
    pub user: Signer<'info>,

    /// Protocol config.
    #[account(mut, seeds = [b"config"], bump)]
    pub config: Account<'info, GlobalConfig>,

    /// Borrower's reputation profile.
    #[account(
        mut,
        seeds = [b"kosh_profile", user.key().as_ref()],
        bump,
        constraint = profile.user == user.key()
    )]
    pub profile: Account<'info, UserReputationProfile>,

    /// Loan account — created fresh for each loan.
    ///
    /// `init` (NOT `init_if_needed`) prevents re-using a loan ID to
    /// re-activate an already-repaid or defaulted loan.
    #[account(
        init,
        payer = user,
        space = LoanAccount::LEN,
        seeds = [b"loan", user.key().as_ref(), &loan_id.to_le_bytes()],
        bump
    )]
    pub loan: Account<'info, LoanAccount>,

    /// Protocol lamport vault (PDA).
    /// CHECK: Verified by seeds — we only transfer lamports out of it.
    #[account(mut, seeds = [b"kosh_vault"], bump)]
    pub vault: AccountInfo<'info>,

    /// Pyth SOL/USD price account — used for TWAP circuit breaker.
    ///
    /// WHY NOT SPOT PRICE: An adversary can sandwich-trade an AMM in the
    /// same slot to temporarily move spot price by several percent,
    /// tripping or bypassing the circuit breaker at will.  TWAP smooths
    /// over short-lived manipulation, making griefing economically
    /// unviable.
    ///
    /// CHECK: Caller supplies the Pyth price account; we validate its
    ///        magic bytes and account type in the handler.
    pub pyth_sol_usd: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}

// ── ProcessRepayment ──────────────────────────────────────────────────────

/// Accounts for `process_repayment`.
/// Compute budget: ~60k CU
#[derive(Accounts)]
#[instruction(loan_id: u64)]
pub struct ProcessRepayment<'info> {
    /// Borrower repaying their loan.
    #[account(mut)]
    pub user: Signer<'info>,

    /// Protocol config — reentrancy guard + pause check.
    #[account(mut, seeds = [b"config"], bump)]
    pub config: Account<'info, GlobalConfig>,

    /// Borrower's reputation profile — active_loan_count decremented.
    #[account(
        mut,
        seeds = [b"kosh_profile", user.key().as_ref()],
        bump,
        constraint = profile.user == user.key()
    )]
    pub profile: Account<'info, UserReputationProfile>,

    /// The loan being repaid.
    ///
    /// We do NOT close the account: the loan record is a permanent audit
    /// trail.  TODO v2: optionally reclaim rent.
    #[account(
        mut,
        seeds = [b"loan", user.key().as_ref(), &loan_id.to_le_bytes()],
        bump,
        constraint = loan.user == user.key(),
        constraint = loan.loan_id == loan_id,
        constraint = loan.status == LoanStatus::Active @ errors::KoshError::OverLeveraged
    )]
    pub loan: Account<'info, LoanAccount>,

    /// User's SPL USDC token account — repayment source.
    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>,

    /// Protocol vault SPL USDC token account — repayment destination.
    #[account(mut)]
    pub vault_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

// ── Program module ────────────────────────────────────────────────────────

#[program]
pub mod kosh {
    use super::*;

    // ── Instruction 1: initialize_config ─────────────────────────────────
    /// One-time protocol bootstrap.  Authority only.
    /// Compute budget: ~10k CU
    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        params: InitializeConfigParams,
    ) -> Result<()> {
        instructions::initialize_config::handler(ctx, params)
    }

    // ── Instruction 2: initialize_user ────────────────────────────────────
    /// Create a reputation profile for a new user.
    /// Compute budget: ~15k CU
    pub fn initialize_user(ctx: Context<InitializeUser>) -> Result<()> {
        instructions::initialize_user::handler(ctx)
    }

    // ── Instruction 3: update_reputation ─────────────────────────────────
    /// Update user reputation via oracle-signed attestation.
    /// This is NOT a ZK proof — see instructions/update_reputation.rs.
    /// Compute budget: up to 250k CU (sha256 + Ed25519 verify)
    pub fn update_reputation(
        ctx: Context<UpdateReputation>,
        oracle_payload: Vec<u8>,
        oracle_sig: [u8; 64],
    ) -> Result<()> {
        instructions::update_reputation::handler(ctx, oracle_payload, oracle_sig)
    }

    // ── Instruction 4: request_loan ───────────────────────────────────────
    /// Issue a loan to the caller based on their reputation score.
    /// Compute budget: ~80k CU
    pub fn request_loan(
        ctx: Context<RequestLoan>,
        loan_id: u64,
        requested_lamports: u64,
        vault_bump: u8,
    ) -> Result<()> {
        instructions::request_loan::handler(ctx, loan_id, requested_lamports, vault_bump)
    }

    // ── Instruction 5: process_repayment ──────────────────────────────────
    /// Accept a USDC repayment for an active loan.
    /// Compute budget: ~60k CU
    pub fn process_repayment(
        ctx: Context<ProcessRepayment>,
        loan_id: u64,
    ) -> Result<()> {
        instructions::process_repayment::handler(ctx, loan_id)
    }
}
