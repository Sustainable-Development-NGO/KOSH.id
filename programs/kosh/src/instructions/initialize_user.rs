use anchor_lang::prelude::*;
use crate::InitializeUser;
use crate::errors::KoshError;

/// Create a fresh reputation profile for `user`.
pub fn handler(ctx: Context<InitializeUser>) -> Result<()> {
    // Check emergency pause first.
    require!(!ctx.accounts.config.emergency_pause, KoshError::ProtocolPaused);

    let profile = &mut ctx.accounts.profile;
    profile.user = ctx.accounts.user.key();
    profile.reputation_score = 0;
    profile.last_update_slot = Clock::get()?.slot;
    profile.oracle_nullifier = [0u8; 32];
    profile.active_loan_count = 0;

    msg!("UserReputationProfile created for {}", profile.user);
    Ok(())
}
