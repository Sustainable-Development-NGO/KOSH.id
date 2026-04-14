/**
 * index.ts — KOSH.id Telegram bot entry point.
 *
 * Commands:
 *  /start   — create Privy embedded wallet + initialize_user on-chain
 *  /credit  — show reputation score and max borrowable in ₹
 *  /verify  — deep-link to TWA for update_reputation flow
 *  /withdraw — post loan Blink URL (WhatsApp-shareable)
 *
 * Error handling: every RPC call has a 10-second timeout.  On failure the
 * user sees a Hindi retry message suitable for 2G connections.
 */

import "dotenv/config";
import { Bot, Context } from "grammy";
import { PublicKey } from "@solana/web3.js";
import {
  fetchReputationProfile,
  fetchVaultBalance,
  maxBorrowableLamports,
  RETRY_MSG_HI,
  connection,
} from "./rpc";
import { getOrCreateWallet } from "./wallet";

// ── Constants ─────────────────────────────────────────────────────────────

/** Approximate SOL/INR rate used for display only. Updated off-chain. */
const SOL_TO_INR = Number(process.env["SOL_TO_INR"] ?? "10000");

/** Lamports per SOL */
const LAMPORTS_PER_SOL = 1_000_000_000n;

/** Blink base URL */
const BLINK_BASE_URL =
  process.env["BLINK_BASE_URL"] ?? "https://kosh.id/api/actions/loan";

/** TWA deep-link for the verification flow */
const TWA_VERIFY_URL =
  process.env["TWA_VERIFY_URL"] ?? "https://t.me/KoshBot/verify";

// ── Bot setup ─────────────────────────────────────────────────────────────

const BOT_TOKEN = process.env["BOT_TOKEN"];
if (!BOT_TOKEN) {
  throw new Error("BOT_TOKEN environment variable is required");
}

const bot = new Bot(BOT_TOKEN);

// ── Helpers ───────────────────────────────────────────────────────────────

function lamportsToInr(lamports: bigint): string {
  const sol = Number(lamports) / Number(LAMPORTS_PER_SOL);
  const inr = sol * SOL_TO_INR;
  return `₹${Math.floor(inr).toLocaleString("en-IN")}`;
}

function getTelegramUserId(ctx: Context): string {
  const id = ctx.from?.id;
  if (!id) throw new Error("Cannot determine Telegram user ID");
  return String(id);
}

/**
 * Wrap any bot handler; catch errors and reply with Hindi retry message.
 */
function withErrorHandler(
  fn: (ctx: Context) => Promise<void>
): (ctx: Context) => Promise<void> {
  return async (ctx: Context) => {
    try {
      await fn(ctx);
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : RETRY_MSG_HI;
      await ctx.reply(msg).catch(() => {
        /* ignore secondary errors */
      });
    }
  };
}

// ── /start ────────────────────────────────────────────────────────────────

bot.command(
  "start",
  withErrorHandler(async (ctx) => {
    const telegramUserId = getTelegramUserId(ctx);
    await ctx.reply("⏳ वॉलेट बना रहे हैं…");

    // 1. Create or retrieve Privy embedded wallet.
    const walletPubkey = await getOrCreateWallet(telegramUserId);

    // 2. Attempt to initialise the on-chain user profile.
    //    The bot cannot sign on behalf of the user; it replies with a
    //    Blink link so the user's embedded wallet signs the transaction.
    const initUrl = `${BLINK_BASE_URL}?action=initialize_user&wallet=${walletPubkey.toBase58()}`;

    await ctx.reply(
      `✅ वॉलेट तैयार है!\n\n` +
        `पता: \`${walletPubkey.toBase58()}\`\n\n` +
        `अपना प्रोफ़ाइल बनाने के लिए नीचे दिए लिंक पर क्लिक करें:\n${initUrl}`,
      { parse_mode: "Markdown" }
    );
  })
);

// ── /credit ───────────────────────────────────────────────────────────────

bot.command(
  "credit",
  withErrorHandler(async (ctx) => {
    const telegramUserId = getTelegramUserId(ctx);
    await ctx.reply("⏳ स्कोर देख रहे हैं…");

    const walletPubkey = await getOrCreateWallet(telegramUserId);
    const profile = await fetchReputationProfile(walletPubkey);

    if (!profile) {
      await ctx.reply(
        "❌ आपका प्रोफ़ाइल नहीं मिला। पहले /start चलाएं।"
      );
      return;
    }

    const currentSlot = BigInt(await connection.getSlot("confirmed"));
    const vaultBalance = await fetchVaultBalance();
    const maxLoan = maxBorrowableLamports(
      profile.reputationScore,
      profile.lastUpdateSlot,
      currentSlot,
      vaultBalance
    );

    await ctx.reply(
      `📊 *आपका KOSH क्रेडिट स्कोर*\n\n` +
        `• स्कोर: ${profile.reputationScore} / 10,000\n` +
        `• अधिकतम लोन: ${lamportsToInr(maxLoan)}\n` +
        `• सक्रिय लोन: ${profile.activeLoanCount}\n\n` +
        `स्कोर अपडेट करने के लिए /verify चलाएं।`,
      { parse_mode: "Markdown" }
    );
  })
);

// ── /verify ───────────────────────────────────────────────────────────────

bot.command(
  "verify",
  withErrorHandler(async (ctx) => {
    await ctx.reply(
      `🔐 *सत्यापन*\n\n` +
        `नीचे दिए लिंक पर क्लिक करके अपना Gig activity सत्यापित करें:\n` +
        `${TWA_VERIFY_URL}\n\n` +
        `(यह आपके ब्राउज़र में खुलेगा और आपके वॉलेट से sign होगा।)`,
      { parse_mode: "Markdown" }
    );
  })
);

// ── /withdraw ─────────────────────────────────────────────────────────────

bot.command(
  "withdraw",
  withErrorHandler(async (ctx) => {
    const telegramUserId = getTelegramUserId(ctx);
    const walletPubkey = await getOrCreateWallet(telegramUserId);
    const profile = await fetchReputationProfile(walletPubkey);

    if (!profile) {
      await ctx.reply("❌ आपका प्रोफ़ाइल नहीं मिला। पहले /start चलाएं।");
      return;
    }

    const currentSlot = BigInt(await connection.getSlot("confirmed"));
    const vaultBalance = await fetchVaultBalance();
    const maxLoan = maxBorrowableLamports(
      profile.reputationScore,
      profile.lastUpdateSlot,
      currentSlot,
      vaultBalance
    );
    const maxInr = lamportsToInr(maxLoan);

    const blinkUrl = `${BLINK_BASE_URL}?wallet=${walletPubkey.toBase58()}&amount=${maxLoan}`;

    await ctx.reply(
      `💸 *लोन लिंक*\n\n` +
        `आप ${maxInr} तक का लोन ले सकते हैं।\n\n` +
        `यह लिंक WhatsApp पर शेयर करें या ब्राउज़र में खोलें:\n${blinkUrl}`,
      { parse_mode: "Markdown" }
    );
  })
);

// ── Start bot ─────────────────────────────────────────────────────────────

bot.start().catch((err) => {
  console.error("Bot crashed:", err);
  process.exit(1);
});
