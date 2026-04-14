//! update_reputation — Oracle-signed attestation handler.
//!
//! IMPORTANT: This instruction verifies an Ed25519 signature produced by the
//! trusted oracle key (GlobalConfig.authority).  This is an *oracle-signed
//! attestation*, NOT a ZK proof.  The oracle signs a payload (e.g., a JSON
//! blob encoding the user's off-chain Gig score) and the user submits that
//! payload + signature on-chain.  We verify the signature using the Solana
//! native Ed25519 program pre-instruction sysvar and derive a reputation score
//! from the payload contents.
//!
//! Real Groth16/PLONK ZK proof verification is a future milestone (TODO v2)
//! that requires a dedicated on-chain verifier program.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::ed25519_program;
use anchor_lang::solana_program::sysvar::instructions as ix_sysvar;
use crate::UpdateReputation;
use crate::errors::KoshError;
use crate::utils::compute_nullifier;
use crate::state::{MAX_REPUTATION, MIN_ORACLE_PAYLOAD_LEN};

/// Update-cooldown in slots (10,000 slots ≈ 83 minutes at 2 slots/sec).
const UPDATE_COOLDOWN_SLOTS: u64 = 10_000;

/// Update the user's reputation score using an oracle-signed attestation.
///
/// # Parameters
/// - `oracle_payload` — raw bytes signed by the oracle.
///   The last 8 bytes are interpreted as a little-endian u64 score in
///   [0, 10_000].  Any additional bytes are opaque context.
/// - `oracle_sig`     — 64-byte Ed25519 signature over `oracle_payload`.
pub fn handler(
    ctx: Context<UpdateReputation>,
    oracle_payload: Vec<u8>,
    oracle_sig: [u8; 64],
) -> Result<()> {
    // ── 1. Check emergency pause ──────────────────────────────────────────
    require!(!ctx.accounts.config.emergency_pause, KoshError::ProtocolPaused);

    // ── 2. Basic payload sanity ───────────────────────────────────────────
    require!(
        oracle_payload.len() >= MIN_ORACLE_PAYLOAD_LEN + 8,
        KoshError::UnauthorizedOracle
    );

    // ── 3. Verify Ed25519 signature ───────────────────────────────────────
    //
    // Solana's native Ed25519 pre-verification program requires that
    // the previous transaction instruction be an Ed25519 instruction.
    // We validate that such an instruction exists in this transaction,
    // signed by the expected oracle key (GlobalConfig.authority).
    let authority_key = ctx.accounts.config.authority;
    verify_ed25519_signature(
        &ctx.accounts.instructions_sysvar,
        &authority_key,
        &oracle_payload,
        &oracle_sig,
    )?;

    // ── 4. Compute and check nullifier (anti-double-spend) ────────────────
    let nullifier = compute_nullifier(&oracle_payload);
    require!(
        ctx.accounts.profile.oracle_nullifier != nullifier,
        KoshError::DuplicateProof
    );

    // ── 5. Enforce update cooldown ────────────────────────────────────────
    let current_slot = Clock::get()?.slot;
    let slots_since_update = current_slot
        .checked_sub(ctx.accounts.profile.last_update_slot)
        .unwrap_or(0);
    require!(
        slots_since_update >= UPDATE_COOLDOWN_SLOTS,
        KoshError::ReputationCooldown
    );

    // ── 6. Extract score from payload (last 8 bytes) ──────────────────────
    //
    // The oracle encodes the score as a little-endian u64 in the final
    // 8 bytes of the payload.  Scores outside [0, MAX_REPUTATION] are
    // clamped so a buggy oracle cannot corrupt state.
    let score_bytes: [u8; 8] = oracle_payload[oracle_payload.len() - 8..]
        .try_into()
        .map_err(|_| error!(KoshError::UnauthorizedOracle))?;
    let raw_score = u64::from_le_bytes(score_bytes);
    let new_score = raw_score.min(MAX_REPUTATION);

    // ── 7. Apply update ───────────────────────────────────────────────────
    let profile = &mut ctx.accounts.profile;
    profile.reputation_score = new_score;
    profile.last_update_slot = current_slot;
    profile.oracle_nullifier = nullifier;

    msg!(
        "Reputation updated for {} → score={} at slot={}",
        profile.user,
        profile.reputation_score,
        current_slot
    );
    Ok(())
}

// ── Ed25519 helper ────────────────────────────────────────────────────────
//
// Anchor does not expose a built-in helper for reading the Ed25519
// pre-instruction sysvar, so we implement it directly following the
// Solana specification:
// https://docs.solana.com/developing/runtime-facilities/programs#ed25519-program
//
// Layout of an Ed25519 instruction data (one signature entry):
//   [0..1]   num_signatures (must be 1)
//   [1..2]   padding
//   [2..4]   signature_offset         (u16 LE)
//   [4..6]   signature_instruction_index (u16 LE)
//   [6..8]   public_key_offset        (u16 LE)
//   [8..10]  public_key_instruction_index (u16 LE)
//   [10..12] message_data_offset      (u16 LE)
//   [12..14] message_data_size        (u16 LE)
//   [14..16] message_instruction_index (u16 LE)
//
// The runtime has already validated the signature before our program runs.
// We only assert that a valid Ed25519 instruction exists for our specific
// (public_key, message) pair.

fn verify_ed25519_signature(
    instructions_sysvar: &AccountInfo,
    expected_pubkey: &Pubkey,
    message: &[u8],
    _sig: &[u8; 64],
) -> Result<()> {
    // Walk all instructions in this transaction looking for an Ed25519
    // pre-instruction whose embedded public key matches authority and
    // whose embedded message matches oracle_payload.
    let mut ix_index: usize = 0;
    loop {
        let ix = match ix_sysvar::load_instruction_at_checked(ix_index, instructions_sysvar) {
            Ok(ix) => ix,
            Err(_) => break, // no more instructions
        };
        ix_index = ix_index.saturating_add(1);

        if ix.program_id != ed25519_program::ID {
            continue;
        }

        let data = &ix.data;
        if data.len() < 2 {
            continue;
        }
        let num_sigs = data[0] as usize;
        for s in 0..num_sigs {
            // Each per-signature header is 14 bytes starting at byte 2 + s*14.
            let base = 2 + s * 14;
            if base + 14 > data.len() {
                break;
            }
            let pubkey_offset =
                u16::from_le_bytes([data[base + 4], data[base + 5]]) as usize;
            let msg_offset =
                u16::from_le_bytes([data[base + 8], data[base + 9]]) as usize;
            let msg_size =
                u16::from_le_bytes([data[base + 10], data[base + 11]]) as usize;

            if pubkey_offset + 32 > data.len() || msg_offset + msg_size > data.len() {
                break;
            }
            let embedded_pubkey = &data[pubkey_offset..pubkey_offset + 32];
            let embedded_msg = &data[msg_offset..msg_offset + msg_size];

            if embedded_pubkey == expected_pubkey.as_ref() && embedded_msg == message {
                return Ok(());
            }
        }
    }

    // No matching Ed25519 pre-instruction found.
    err!(KoshError::UnauthorizedOracle)
}
