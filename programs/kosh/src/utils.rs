/// KOSH.id math utilities — purely integer, no floats.
///
/// All arithmetic uses checked_* variants.  Any overflow returns
/// `None`, which the caller maps to an appropriate program error.

use crate::state::SLOTS_PER_YEAR;

// ── Reputation decay (discrete bit-shift) ─────────────────────────────────
//
// Continuous exponential decay  e^(-λt)  requires floating-point or
// fixed-point pre-computation off-chain because the checked_* integer
// primitives available on-chain cannot represent irrational exponents.
// The discrete approximation below uses right-bit-shift (halving) every
// 10,000 slots (≈ 5,000 seconds), which is intentional and documented:
//
//   decay_steps   = slots_elapsed / 10_000   (integer division)
//   effective_score = reputation_score >> decay_steps
//
// Each step halves the score, so the curve is steeper than e^(-λt) but
// has no floating-point rounding and never overflows.

/// Compute the effective (time-decayed) reputation score.
///
/// Returns `None` on arithmetic overflow (should never happen for
/// realistic inputs, but we propagate the error for correctness).
pub fn effective_score(
    reputation_score: u64,
    last_update_slot: u64,
    current_slot: u64,
) -> Option<u64> {
    let slots_elapsed = current_slot.checked_sub(last_update_slot)?;
    // Each decay step is 10,000 slots.
    let decay_steps = slots_elapsed / 10_000;
    // Cap shift to 63 to avoid UB; beyond 63 the score is effectively 0.
    let shift = decay_steps.min(63) as u32;
    Some(reputation_score >> shift)
}

/// Compute the maximum borrowable lamports given an effective score and
/// the current vault balance.
///
/// max_borrowable = (effective_score * vault_balance) / MAX_REPUTATION
///
/// Returns `None` on overflow (vault_balance would need to be
/// astronomically large for this to happen, but we guard anyway).
pub fn max_borrowable_lamports(
    effective_score: u64,
    vault_balance: u64,
    max_reputation: u64,
) -> Option<u64> {
    let numerator = effective_score.checked_mul(vault_balance)?;
    numerator.checked_div(max_reputation)
}

// ── Simple-interest accrual (per-slot) ────────────────────────────────────
//
// interest = principal * interest_rate_bps / 10_000 * slots_elapsed / SLOTS_PER_YEAR
//
// We divide by 10_000 BEFORE multiplying by slots_elapsed to avoid u64
// overflow.  For realistic inputs (principal ≤ ~1.8e10 lamports ≈ 18 SOL
// at u64 max / bps-max 10_000 / slots_per_year 78_840_000) this ordering
// keeps all intermediate values within u64 range.
//
// SLOTS_PER_YEAR = 78_840_000  (≈ 2 slots/sec × 31,536,000 sec/year)
//
// No compounding — v1 keeps repayment math simple for users.

/// Compute accrued interest in lamports (rounded down).
///
/// Returns `None` on overflow.
pub fn accrued_interest(
    principal_lamports: u64,
    interest_rate_bps: u16,
    slots_elapsed: u64,
) -> Option<u64> {
    // Step 1: principal * bps / 10_000  (base annual interest lamports)
    let annual_base = principal_lamports
        .checked_mul(interest_rate_bps as u64)?
        .checked_div(10_000)?;
    // Step 2: annual_base * slots_elapsed / SLOTS_PER_YEAR
    annual_base
        .checked_mul(slots_elapsed)?
        .checked_div(SLOTS_PER_YEAR)
}

// ── Nullifier ──────────────────────────────────────────────────────────────

use sha2::{Digest, Sha256};

/// Compute SHA-256 of `data` and return the 32-byte digest.
/// This is used as the oracle-payload nullifier — NOT a ZK proof.
/// Real Groth16/PLONK proof verification is a TODO v2 milestone that
/// requires a dedicated on-chain verifier program.
pub fn compute_nullifier(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effective_score_no_decay() {
        // No time elapsed → score unchanged.
        assert_eq!(effective_score(5_000, 100, 100), Some(5_000));
    }

    #[test]
    fn test_effective_score_one_step() {
        // 10,000 slots elapsed → one halving.
        assert_eq!(effective_score(5_000, 0, 10_000), Some(2_500));
    }

    #[test]
    fn test_effective_score_two_steps() {
        assert_eq!(effective_score(8_000, 0, 20_000), Some(2_000));
    }

    #[test]
    fn test_effective_score_large_decay() {
        // 64 steps would shift out entirely; we cap at 63.
        assert_eq!(effective_score(10_000, 0, 640_000), Some(0));
    }

    #[test]
    fn test_max_borrowable() {
        // Half score → half vault.
        assert_eq!(
            max_borrowable_lamports(5_000, 1_000_000, 10_000),
            Some(500_000)
        );
    }

    #[test]
    fn test_accrued_interest_zero_slots() {
        assert_eq!(accrued_interest(1_000_000, 500, 0), Some(0));
    }

    #[test]
    fn test_accrued_interest_one_year() {
        // 5% APR (500 bps) over one year on 1 SOL (1e9 lamports).
        // Expected ≈ 50_000 lamports (rounding down from integer div).
        let interest = accrued_interest(1_000_000_000, 500, SLOTS_PER_YEAR).unwrap();
        assert_eq!(interest, 50_000_000); // 5% of 1 SOL
    }

    #[test]
    fn test_nullifier_deterministic() {
        let a = compute_nullifier(b"hello");
        let b = compute_nullifier(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn test_nullifier_unique() {
        let a = compute_nullifier(b"hello");
        let b = compute_nullifier(b"world");
        assert_ne!(a, b);
    }
}
