/**
 * rpc.ts — Solana RPC helpers for KOSH.id bot.
 *
 * All RPC calls enforce a 10-second timeout and return a user-readable
 * Hindi error message on failure, suitable for 2G network conditions.
 */

import {
  Connection,
  PublicKey,
  Transaction,
  TransactionInstruction,
  clusterApiUrl,
} from "@solana/web3.js";

// ── Retry config ──────────────────────────────────────────────────────────

/** Human-readable Hindi retry message shown to the user on any RPC error. */
export const RETRY_MSG_HI =
  "कुछ गलत हुआ, दोबारा कोशिश करें 🙏";

/** Timeout for every RPC call in milliseconds (10 seconds). */
const RPC_TIMEOUT_MS = 10_000;

// ── Connection singleton ──────────────────────────────────────────────────

const HELIUS_RPC_URL =
  process.env["HELIUS_RPC_URL"] ?? clusterApiUrl("devnet");

export const connection = new Connection(HELIUS_RPC_URL, {
  commitment: "confirmed",
  confirmTransactionInitialTimeout: RPC_TIMEOUT_MS,
  disableRetryOnRateLimit: false,
});

// ── PDA helpers ───────────────────────────────────────────────────────────

const PROGRAM_ID = new PublicKey(
  process.env["KOSH_PROGRAM_ID"] ??
    "KoshVau1tXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"
);

/** Derive the GlobalConfig PDA. */
export function configPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([Buffer.from("config")], PROGRAM_ID);
}

/** Derive a UserReputationProfile PDA. */
export function profilePda(user: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("kosh_profile"), user.toBuffer()],
    PROGRAM_ID
  );
}

// ── Account fetch with timeout ────────────────────────────────────────────

/**
 * Fetch a raw account, enforcing a 10-second timeout.
 * Returns `null` if the account does not exist.
 * Throws a localised Hindi error string on network failure.
 */
export async function fetchAccountWithTimeout(
  address: PublicKey
): Promise<Buffer | null> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), RPC_TIMEOUT_MS);
  try {
    const accountInfo = await connection.getAccountInfo(address, "confirmed");
    clearTimeout(timer);
    return accountInfo?.data ?? null;
  } catch (err) {
    clearTimeout(timer);
    throw new Error(RETRY_MSG_HI);
  }
}

// ── Reputation profile parser ─────────────────────────────────────────────

export interface ReputationProfile {
  user: string;
  reputationScore: bigint;
  lastUpdateSlot: bigint;
  oracleNullifier: Uint8Array;
  activeLoanCount: number;
}

/**
 * Deserialise a UserReputationProfile from raw account data.
 *
 * Layout (matches state.rs):
 *   [0..8]   discriminator
 *   [8..40]  user (Pubkey)
 *   [40..48] reputation_score (u64 LE)
 *   [48..56] last_update_slot (u64 LE)
 *   [56..88] oracle_nullifier ([u8; 32])
 *   [88]     active_loan_count (u8)
 */
export function parseReputationProfile(data: Buffer): ReputationProfile {
  if (data.length < 89) throw new Error("Invalid profile account size");
  const user = new PublicKey(data.slice(8, 40)).toBase58();
  const reputationScore = data.readBigUInt64LE(40);
  const lastUpdateSlot = data.readBigUInt64LE(48);
  const oracleNullifier = new Uint8Array(data.slice(56, 88));
  const activeLoanCount = data[88]!;
  return { user, reputationScore, lastUpdateSlot, oracleNullifier, activeLoanCount };
}

/**
 * Fetch and parse the reputation profile for a user.
 *
 * Returns `null` if the account has not been initialised yet.
 */
export async function fetchReputationProfile(
  user: PublicKey
): Promise<ReputationProfile | null> {
  const [pda] = profilePda(user);
  const data = await fetchAccountWithTimeout(pda);
  if (!data) return null;
  return parseReputationProfile(data);
}

// ── Max-borrowable (mirrors on-chain discrete decay) ─────────────────────

const MAX_REPUTATION = 10_000n;

/**
 * Compute the effective reputation score after discrete bit-shift decay.
 * Mirrors the on-chain formula in utils.rs exactly.
 */
export function effectiveScore(
  reputationScore: bigint,
  lastUpdateSlot: bigint,
  currentSlot: bigint
): bigint {
  const slotsElapsed = currentSlot > lastUpdateSlot
    ? currentSlot - lastUpdateSlot
    : 0n;
  const decaySteps = slotsElapsed / 10_000n;
  const shift = decaySteps < 64n ? decaySteps : 63n;
  return reputationScore >> shift;
}

/**
 * Compute max borrowable lamports given effective score and vault balance.
 */
export function maxBorrowableLamports(
  reputationScore: bigint,
  lastUpdateSlot: bigint,
  currentSlot: bigint,
  vaultBalance: bigint
): bigint {
  const eff = effectiveScore(reputationScore, lastUpdateSlot, currentSlot);
  return (eff * vaultBalance) / MAX_REPUTATION;
}

/**
 * Fetch the vault lamport balance.
 */
export async function fetchVaultBalance(): Promise<bigint> {
  const [vaultPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("kosh_vault")],
    PROGRAM_ID
  );
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), RPC_TIMEOUT_MS);
  try {
    const balance = await connection.getBalance(vaultPda, "confirmed");
    clearTimeout(timer);
    return BigInt(balance);
  } catch {
    clearTimeout(timer);
    throw new Error(RETRY_MSG_HI);
  }
}
