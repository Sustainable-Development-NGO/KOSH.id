/**
 * wallet.ts — Privy embedded-wallet integration for KOSH.id bot.
 *
 * Creates a server-side embedded wallet for each Telegram user via the
 * Privy API.  The wallet public key is used as the Solana signer for
 * on-chain instructions.
 *
 * TODO v2: Replace with a full Privy TWA flow once the mini-app is live.
 */

import { PrivyClient } from "@privy-io/server-auth";
import { PublicKey } from "@solana/web3.js";
import { RETRY_MSG_HI } from "./rpc";

const PRIVY_APP_ID = process.env["PRIVY_APP_ID"] ?? "";
const PRIVY_APP_SECRET = process.env["PRIVY_APP_SECRET"] ?? "";

let _privy: PrivyClient | null = null;

function getPrivyClient(): PrivyClient {
  if (!_privy) {
    _privy = new PrivyClient(PRIVY_APP_ID, PRIVY_APP_SECRET);
  }
  return _privy;
}

/**
 * Create (or retrieve) a Privy embedded wallet for a Telegram user.
 *
 * @param telegramUserId  Stable Telegram user ID (string).
 * @returns               The Solana public key of the embedded wallet.
 */
export async function getOrCreateWallet(
  telegramUserId: string
): Promise<PublicKey> {
  const privy = getPrivyClient();
  try {
    // Privy uses the external ID to idempotently create one wallet per user.
    const user = await privy.importUser({
      linkedAccounts: [
        {
          type: "telegram",
          telegramUserId,
          // first_name/username are optional but improve Privy dashboard UX.
        },
      ],
      createEthereumWallet: false,
      createSolanaWallet: true,
    });

    const solanaWallet = user.linkedAccounts.find(
      (a: { type: string }) => a.type === "wallet" && (a as any).chainType === "solana"
    ) as { address: string } | undefined;

    if (!solanaWallet) {
      throw new Error("Privy did not create a Solana wallet");
    }

    return new PublicKey(solanaWallet.address);
  } catch (err) {
    // If the error is already the localised Hindi message, re-throw as-is.
    // Otherwise wrap it so users always see a friendly retry prompt.
    if (err instanceof Error && err.message === RETRY_MSG_HI) {
      throw err;
    }
    throw new Error(RETRY_MSG_HI);
  }
}
