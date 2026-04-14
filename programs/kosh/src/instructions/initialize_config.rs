use anchor_lang::prelude::*;
use crate::{InitializeConfig, InitializeConfigParams};

/// One-time protocol bootstrap.
///
/// `emergency_pause` starts as `false`; authority can set it later via
/// a dedicated pause instruction (TODO v2).
pub fn handler(ctx: Context<InitializeConfig>, params: InitializeConfigParams) -> Result<()> {
    // emergency_pause is not relevant at init time — the account does not
    // exist yet.  No protocol-pause check needed here.

    let config = &mut ctx.accounts.config;
    config.authority = ctx.accounts.authority.key();
    config.protocol_fee_bps = params.protocol_fee_bps;
    config.base_interest_bps = params.base_interest_bps;
    config.emergency_pause = false;
    config.min_reputation_for_loan = params.min_reputation_for_loan;
    config.twap_freeze_threshold_bps = params.twap_freeze_threshold_bps;
    config.reentrancy_lock = false;

    msg!(
        "GlobalConfig initialised. authority={} fee_bps={} base_interest_bps={}",
        config.authority,
        config.protocol_fee_bps,
        config.base_interest_bps
    );
    Ok(())
}
