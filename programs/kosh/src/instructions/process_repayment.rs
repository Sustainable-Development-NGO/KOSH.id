use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};
use crate::ProcessRepayment;
use crate::state::LoanStatus;
use crate::errors::KoshError;

/// Accept a USDC repayment and mark the loan Repaid.
pub fn handler(ctx: Context<ProcessRepayment>, loan_id: u64) -> Result<()> {
    // ── 1. Reentrancy guard ───────────────────────────────────────────────
    require!(!ctx.accounts.config.reentrancy_lock, KoshError::ReentrancyGuard);

    // ── 2. Emergency pause ────────────────────────────────────────────────
    require!(!ctx.accounts.config.emergency_pause, KoshError::ProtocolPaused);

    // ── 3. Compute repayment amount (principal + simple interest) ─────────
    let current_slot = Clock::get()?.slot;
    let slots_elapsed = current_slot
        .checked_sub(ctx.accounts.loan.start_slot)
        .unwrap_or(0);

    let interest = crate::utils::accrued_interest(
        ctx.accounts.loan.principal_lamports,
        ctx.accounts.loan.interest_rate_bps,
        slots_elapsed,
    )
    .ok_or(error!(KoshError::InsufficientVault))?;

    // Repayment in USDC micro-units (6 decimals).
    // We treat 1 lamport ≈ 1 USDC micro-unit for v1 simplicity.
    // TODO v2: apply on-chain SOL/USD price conversion.
    let total_repayment = ctx
        .accounts
        .loan
        .principal_lamports
        .checked_add(interest)
        .ok_or(error!(KoshError::InsufficientVault))?;

    // ── 4. Set reentrancy lock ────────────────────────────────────────────
    ctx.accounts.config.reentrancy_lock = true;

    // ── 5. SPL token transfer: user → vault ───────────────────────────────
    let cpi_accounts = Transfer {
        from: ctx.accounts.user_token_account.to_account_info(),
        to: ctx.accounts.vault_token_account.to_account_info(),
        authority: ctx.accounts.user.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
    token::transfer(cpi_ctx, total_repayment).map_err(|e| {
        ctx.accounts.config.reentrancy_lock = false;
        e
    })?;

    // ── 6. Clear reentrancy lock ──────────────────────────────────────────
    ctx.accounts.config.reentrancy_lock = false;

    // ── 7. Mark loan Repaid ───────────────────────────────────────────────
    ctx.accounts.loan.status = LoanStatus::Repaid;

    // ── 8. Decrement active loan count ────────────────────────────────────
    ctx.accounts.profile.active_loan_count = ctx
        .accounts
        .profile
        .active_loan_count
        .saturating_sub(1);

    msg!(
        "Loan {} repaid: principal={} interest={} total={}",
        loan_id,
        ctx.accounts.loan.principal_lamports,
        interest,
        total_repayment
    );
    Ok(())
}
