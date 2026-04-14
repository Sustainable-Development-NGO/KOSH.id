/**
 * action.ts — KOSH.id Solana Actions (Blink) endpoint.
 *
 * GET  /api/actions/loan  — return Actions metadata + max borrowable amount
 * POST /api/actions/loan  — build + return a VersionedTransaction for the
 *                           user's wallet to sign.
 *
 * The user's wallet signs the transaction client-side via their embedded
 * Privy/Phantom wallet.  This server NEVER holds private keys.
 *
 * Spec: https://solana.com/docs/advanced/actions
 */

import "dotenv/config";
import {
  Connection,
  PublicKey,
  TransactionMessage,
  VersionedTransaction,
  TransactionInstruction,
  SystemProgram,
  clusterApiUrl,
} from "@solana/web3.js";

// ── Config ────────────────────────────────────────────────────────────────

const HELIUS_RPC_URL =
  process.env["HELIUS_RPC_URL"] ?? clusterApiUrl("devnet");

const PROGRAM_ID = new PublicKey(
  process.env["KOSH_PROGRAM_ID"] ??
    "KoshVau1tXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"
);

/** Approximate SOL/INR conversion for display. */
const SOL_TO_INR = Number(process.env["SOL_TO_INR"] ?? "10000");
const LAMPORTS_PER_SOL = 1_000_000_000n;

const connection = new Connection(HELIUS_RPC_URL, {
  commitment: "confirmed",
  confirmTransactionInitialTimeout: 10_000,
});

// ── PDA helpers ───────────────────────────────────────────────────────────

function profilePda(user: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("kosh_profile"), user.toBuffer()],
    PROGRAM_ID
  );
}

function vaultPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("kosh_vault")],
    PROGRAM_ID
  );
}

function loanPda(user: PublicKey, loanId: bigint): [PublicKey, number] {
  const loanIdBuf = Buffer.allocUnsafe(8);
  loanIdBuf.writeBigUInt64LE(loanId);
  return PublicKey.findProgramAddressSync(
    [Buffer.from("loan"), user.toBuffer(), loanIdBuf],
    PROGRAM_ID
  );
}

function configPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([Buffer.from("config")], PROGRAM_ID);
}

// ── Discrete decay (mirrors utils.rs exactly) ─────────────────────────────

const MAX_REPUTATION = 10_000n;

function effectiveScore(
  reputationScore: bigint,
  lastUpdateSlot: bigint,
  currentSlot: bigint
): bigint {
  const slotsElapsed =
    currentSlot > lastUpdateSlot ? currentSlot - lastUpdateSlot : 0n;
  const decaySteps = slotsElapsed / 10_000n;
  const shift = decaySteps < 64n ? decaySteps : 63n;
  return reputationScore >> shift;
}

function maxBorrowableLamports(
  reputationScore: bigint,
  lastUpdateSlot: bigint,
  currentSlot: bigint,
  vaultBalance: bigint
): bigint {
  const eff = effectiveScore(reputationScore, lastUpdateSlot, currentSlot);
  return (eff * vaultBalance) / MAX_REPUTATION;
}

// ── Profile parser ────────────────────────────────────────────────────────

interface ReputationProfile {
  reputationScore: bigint;
  lastUpdateSlot: bigint;
  activeLoanCount: number;
}

function parseProfile(data: Buffer): ReputationProfile {
  if (data.length < 89) throw new Error("Invalid profile size");
  return {
    reputationScore: data.readBigUInt64LE(40),
    lastUpdateSlot: data.readBigUInt64LE(48),
    activeLoanCount: data[88]!,
  };
}

async function fetchProfile(
  user: PublicKey
): Promise<ReputationProfile | null> {
  const [pda] = profilePda(user);
  const info = await connection.getAccountInfo(pda, "confirmed");
  if (!info) return null;
  return parseProfile(info.data as Buffer);
}

// ── Display helpers ───────────────────────────────────────────────────────

function lamportsToInr(lamports: bigint): string {
  const sol = Number(lamports) / Number(LAMPORTS_PER_SOL);
  const inr = Math.floor(sol * SOL_TO_INR);
  return inr.toLocaleString("en-IN");
}

// ── Instruction builder ───────────────────────────────────────────────────
//
// Build a `request_loan` instruction manually.  In production this should
// use the generated IDL client.  We use raw instruction data here to keep
// the blink package dependency-free of the Anchor client library.
//
// Instruction discriminator = sha256("global:request_loan")[0..8]
// (computed offline; value below matches the IDL)

const REQUEST_LOAN_DISCRIMINATOR = Buffer.from([
  // sha256("global:request_loan")[0..8] — placeholder, replace with IDL value
  0xd3, 0x34, 0xd5, 0x47, 0x82, 0x7a, 0x1e, 0x9b,
]);

function buildRequestLoanInstruction(
  user: PublicKey,
  loanId: bigint,
  requestedLamports: bigint,
  vaultBump: number
): TransactionInstruction {
  const [configKey] = configPda();
  const [profileKey] = profilePda(user);
  const [loanKey] = loanPda(user, loanId);
  const [vaultKey] = vaultPda();

  // Instruction data: discriminator(8) + loan_id(8) + requested_lamports(8) + vault_bump(1)
  const data = Buffer.allocUnsafe(8 + 8 + 8 + 1);
  REQUEST_LOAN_DISCRIMINATOR.copy(data, 0);
  data.writeBigUInt64LE(loanId, 8);
  data.writeBigUInt64LE(requestedLamports, 16);
  data.writeUInt8(vaultBump, 24);

  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: user, isSigner: true, isWritable: true },
      { pubkey: configKey, isSigner: false, isWritable: true },
      { pubkey: profileKey, isSigner: false, isWritable: true },
      { pubkey: loanKey, isSigner: false, isWritable: true },
      { pubkey: vaultKey, isSigner: false, isWritable: true },
      // pyth_sol_usd — caller must supply the correct devnet/mainnet address.
      // For now we use a placeholder; production clients set via env.
      {
        pubkey: new PublicKey(
          process.env["PYTH_SOL_USD_PRICE_ACCOUNT"] ??
            "H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG"
        ),
        isSigner: false,
        isWritable: false,
      },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data,
  });
}

// ── GET handler ───────────────────────────────────────────────────────────

export interface GetActionResponse {
  title: string;
  icon: string;
  description: string;
  label: string;
  links: {
    actions: Array<{
      label: string;
      href: string;
    }>;
  };
}

/**
 * GET /api/actions/loan?wallet=<base58>
 *
 * Returns Solana Actions metadata including the max borrowable amount
 * displayed in ₹.
 */
export async function getLoanAction(
  walletAddress: string
): Promise<GetActionResponse> {
  const user = new PublicKey(walletAddress);
  const profile = await fetchProfile(user);

  let maxLoan = 0n;
  if (profile && profile.activeLoanCount === 0) {
    const currentSlot = BigInt(await connection.getSlot("confirmed"));
    const [vaultKey] = vaultPda();
    const vaultBalance = BigInt(
      await connection.getBalance(vaultKey, "confirmed")
    );
    maxLoan = maxBorrowableLamports(
      profile.reputationScore,
      profile.lastUpdateSlot,
      currentSlot,
      vaultBalance
    );
  }

  const amountInr = lamportsToInr(maxLoan);

  return {
    title: "KOSH Credit",
    icon: "https://kosh.id/logo.png",
    description: `You qualify for ₹${amountInr} based on your verified activity.`,
    label: `Claim ₹${amountInr}`,
    links: {
      actions: [
        {
          label: `Claim ₹${amountInr}`,
          href: `/api/actions/loan?amount=${maxLoan}&wallet=${walletAddress}`,
        },
      ],
    },
  };
}

// ── POST handler ──────────────────────────────────────────────────────────

export interface PostActionResponse {
  transaction: string; // base64-encoded VersionedTransaction
  message: string;
}

/**
 * POST /api/actions/loan?amount=<lamports>&wallet=<base58>
 *
 * Builds a VersionedTransaction calling `request_loan` and returns it
 * base64-encoded.  The user's embedded wallet signs it client-side.
 * This server never holds or touches any private key.
 */
export async function postLoanAction(
  walletAddress: string,
  amount: bigint,
  loanId: bigint
): Promise<PostActionResponse> {
  const user = new PublicKey(walletAddress);
  const [, vaultBump] = vaultPda();

  const ix = buildRequestLoanInstruction(user, loanId, amount, vaultBump);

  const { blockhash } = await connection.getLatestBlockhash("confirmed");
  const message = new TransactionMessage({
    payerKey: user,
    recentBlockhash: blockhash,
    instructions: [ix],
  }).compileToV0Message();

  const tx = new VersionedTransaction(message);
  // NOTE: We do NOT sign here.  The Blink client (user's wallet) signs.
  const base64Tx = Buffer.from(tx.serialize()).toString("base64");

  return {
    transaction: base64Tx,
    message: `Loan of ₹${lamportsToInr(amount)} requested. Please approve in your wallet.`,
  };
}
