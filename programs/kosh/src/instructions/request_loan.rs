//! request_loan — Issue a new SOL loan based on the user's reputation score.
//!
//! Security notes:
//!
//! TWAP circuit breaker:
//!   We read the Pyth SOL/USD price account and check the 1-hour EMA.
//!   We use EMA (TWAP), never the spot price.  Spot-price griefing vector:
//!   an adversary submits a large swap on an AMM in the same slot, moving
//!   the spot price by several percent — enough to trip or bypass the
//!   circuit breaker at will.  EMA smooths over short-lived manipulation
//!   windows, making griefing economically unviable.
//!
//! Reentrancy:
//!   Solana is single-threaded; true concurrent reentrancy (EVM-style) is
//!   impossible.  However a malicious program invoked via CPI CAN call back
//!   into this program within the same transaction.  The reentrancy_lock
//!   bool guards against this: we set it true before any CPI and check it
//!   is false on every entry.  Unlike EVM, there is no separate call stack
//!   frame to protect — the lock is explicit state.

use anchor_lang::prelude::*;
use crate::RequestLoan;
use crate::state::{LoanStatus, MAX_REPUTATION};
use crate::errors::KoshError;
use crate::utils::{effective_score, max_borrowable_lamports};

/// Maximum acceptable Pyth price account age in slots (≈60 sec).
const PYTH_MAX_AGE_SLOTS: u64 = 120;

/// Pyth v2 account magic number.
const PYTH_MAGIC: u32 = 0xa1b2c3d4;
/// Pyth account type for price accounts.
const PYTH_PRICE_ATYPE: u32 = 3;

/// Issue a new loan to `user` if all safety checks pass.
pub fn handler(
    ctx: Context<RequestLoan>,
    loan_id: u64,
    requested_lamports: u64,
    vault_bump: u8,
) -> Result<()> {
    // ── 1. Reentrancy guard (entry check) ─────────────────────────────────
    require!(!ctx.accounts.config.reentrancy_lock, KoshError::ReentrancyGuard);

    // ── 2. Emergency pause ────────────────────────────────────────────────
    require!(!ctx.accounts.config.emergency_pause, KoshError::ProtocolPaused);

    // ── 3. TWAP circuit breaker ───────────────────────────────────────────
    check_twap_circuit_breaker(
        &ctx.accounts.pyth_sol_usd,
        ctx.accounts.config.twap_freeze_threshold_bps,
    )?;

    // ── 4. No stacking loans (v1 policy) ──────────────────────────────────
    require!(
        ctx.accounts.profile.active_loan_count == 0,
        KoshError::OverLeveraged
    );

    // ── 5. Minimum reputation check ───────────────────────────────────────
    let current_slot = Clock::get()?.slot;
    let eff_score = effective_score(
        ctx.accounts.profile.reputation_score,
        ctx.accounts.profile.last_update_slot,
        current_slot,
    )
    .ok_or(error!(KoshError::InsufficientVault))?;

    require!(
        eff_score >= ctx.accounts.config.min_reputation_for_loan,
        KoshError::InsufficientVault
    );

    // ── 6. Compute max borrowable via discrete decay ──────────────────────
    //
    // Continuous e^(-λt) is unsuitable on-chain: checked integer math
    // cannot represent irrational exponents.  We use discrete bit-shift
    // decay (documented in utils.rs) as an intentional approximation.
    let vault_balance = ctx.accounts.vault.lamports();
    let max_loan = max_borrowable_lamports(eff_score, vault_balance, MAX_REPUTATION)
        .ok_or(error!(KoshError::InsufficientVault))?;

    require!(requested_lamports <= max_loan, KoshError::InsufficientVault);
    require!(vault_balance >= requested_lamports, KoshError::InsufficientVault);

    // ── 7. Set reentrancy lock before CPI ─────────────────────────────────
    ctx.accounts.config.reentrancy_lock = true;

    // ── 8. Transfer lamports from vault PDA to user ───────────────────────
    let vault_seeds: &[&[u8]] = &[b"kosh_vault", &[vault_bump]];
    let signer_seeds = &[vault_seeds];
    anchor_lang::solana_program::program::invoke_signed(
        &anchor_lang::solana_program::system_instruction::transfer(
            ctx.accounts.vault.key,
            ctx.accounts.user.key,
            requested_lamports,
        ),
        &[
            ctx.accounts.vault.to_account_info(),
            ctx.accounts.user.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
        ],
        signer_seeds,
    )
    .map_err(|e| {
        ctx.accounts.config.reentrancy_lock = false;
        e
    })?;

    // ── 9. Clear reentrancy lock ──────────────────────────────────────────
    ctx.accounts.config.reentrancy_lock = false;

    // ── 10. Write loan account ────────────────────────────────────────────
    let loan = &mut ctx.accounts.loan;
    loan.user = ctx.accounts.user.key();
    loan.loan_id = loan_id;
    loan.principal_lamports = requested_lamports;
    loan.interest_rate_bps = ctx.accounts.config.base_interest_bps;
    loan.start_slot = current_slot;
    loan.status = LoanStatus::Active;

    // ── 11. Increment active loan count ──────────────────────────────────
    ctx.accounts.profile.active_loan_count = ctx
        .accounts
        .profile
        .active_loan_count
        .checked_add(1)
        .ok_or(error!(KoshError::OverLeveraged))?;

    msg!(
        "Loan {} issued: {} lamports at {} bps, score={}, slot={}",
        loan_id,
        requested_lamports,
        loan.interest_rate_bps,
        eff_score,
        current_slot
    );
    Ok(())
}

// ── Pyth TWAP helper ──────────────────────────────────────────────────────
//
// We parse Pyth price account data directly rather than using pyth-sdk-solana
// to avoid a dependency conflict: pyth-sdk-solana 0.10 requires
// `solana-account-info` v2, but Anchor 0.30.1 uses `solana-program` v1.18
// which exposes a different AccountInfo type.  Direct parsing is equally
// safe — the Pyth v2 account layout is stable and well-documented.
//
// Pyth v2 Price Account layout (little-endian):
//   [  0.. 4] magic:       u32  = 0xa1b2c3d4
//   [  4.. 8] ver:         u32
//   [  8..12] atype:       u32  = 3 (price account)
//   [ 12..16] size:        u32
//   [ 16..20] price_type:  u32
//   [ 20..24] exponent:    i32
//   [ 24..28] num:         u32
//   [ 28..32] num_qt:      u32
//   [ 32..40] last_slot:   u64
//   [ 40..48] valid_slot:  u64
//   — EMA struct { val: i64, numer: i64, denom: i64 } = 24 bytes —
//   [ 48..56] ema_price:   i64   ← TWAP price
//   [ 56..64] ema_numer:   i64
//   [ 64..72] ema_denom:   i64
//   — EMA confidence (same layout) —
//   [ 72..80] ema_conf:    i64   ← TWAP confidence interval

fn check_twap_circuit_breaker(
    pyth_account: &AccountInfo,
    threshold_bps: u16,
) -> Result<()> {
    let data = pyth_account
        .try_borrow_data()
        .map_err(|_| error!(KoshError::CircuitBreakerTripped))?;

    // Minimum length: 80 bytes to reach ema_conf field.
    if data.len() < 80 {
        return err!(KoshError::CircuitBreakerTripped);
    }

    // Validate magic and account type.
    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let atype = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if magic != PYTH_MAGIC || atype != PYTH_PRICE_ATYPE {
        return err!(KoshError::CircuitBreakerTripped);
    }

    // Freshness check — valid_slot must be recent.
    let valid_slot = u64::from_le_bytes(data[40..48].try_into().unwrap());
    let current_slot = Clock::get()?.slot;
    if current_slot.saturating_sub(valid_slot) > PYTH_MAX_AGE_SLOTS {
        return err!(KoshError::CircuitBreakerTripped);
    }

    // Read TWAP price and confidence interval.
    let ema_price_raw = i64::from_le_bytes(data[48..56].try_into().unwrap());
    let ema_conf_raw  = i64::from_le_bytes(data[72..80].try_into().unwrap());

    let price_abs = ema_price_raw.unsigned_abs();
    let conf_abs  = ema_conf_raw.unsigned_abs();

    if price_abs == 0 {
        return err!(KoshError::CircuitBreakerTripped);
    }

    // conf_bps = conf * 10_000 / price
    let conf_bps = conf_abs
        .checked_mul(10_000)
        .and_then(|v| v.checked_div(price_abs))
        .ok_or(error!(KoshError::CircuitBreakerTripped))?;

    require!(
        conf_bps <= threshold_bps as u64,
        KoshError::CircuitBreakerTripped
    );

    Ok(())
}
