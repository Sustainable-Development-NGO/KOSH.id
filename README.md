# KOSH.id

KOSH.id is a Solana-based micro-lending protocol focused on gig workers, with:
- an on-chain Anchor program (`programs/kosh`) for reputation-based lending,
- a Telegram bot (`bot`) for user interaction in Hindi,
- and a Solana Actions/Blink backend (`blink`) for wallet-signed loan flows.

---

## Repository Structure

```text
KOSH.id/
├─ programs/kosh/        # Anchor smart contract (core protocol)
├─ bot/                  # Telegram bot (TypeScript)
├─ blink/                # Solana Actions/Blink service (TypeScript)
├─ Anchor.toml
├─ Cargo.toml
└─ README.md
```

---

## Core Protocol (Anchor Program)

### Program Basics
- **Crate:** `programs/kosh`
- **Framework:** Anchor `0.30.1`
- **Program ID in source:** `Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS` (Anchor placeholder in `lib.rs`)

### Main Accounts
- `GlobalConfig` (PDA seed: `["config"]`)
  - authority, fee/interest params, pause flag, reputation threshold, TWAP threshold, reentrancy lock
- `UserReputationProfile` (PDA seed: `["kosh_profile", user]`)
  - reputation score, last update slot, oracle nullifier, active loan count
- `LoanAccount` (PDA seed: `["loan", user, loan_id_le_bytes]`)
  - principal, interest rate, start slot, loan status

### Instructions
1. `initialize_config`
   - one-time protocol bootstrap by authority.
2. `initialize_user`
   - creates borrower profile PDA.
3. `update_reputation`
   - applies oracle-signed attestation (Ed25519 pre-instruction verification).
4. `request_loan`
   - checks reputation and risk controls, then issues SOL loan from vault PDA.
5. `process_repayment`
   - repays in SPL token flow and marks loan as repaid.

### Key Protocol Rules
- v1 policy: **one active loan per user**.
- Reputation uses discrete decay over slots (`score >> decay_steps` with 10,000-slot steps).
- Borrowing cap is proportional to effective score:
  - `max_borrowable = effective_score * vault_balance / MAX_REPUTATION`
- Interest accrues as simple per-slot interest (no compounding in v1).

### Security Controls
- Emergency pause (`GlobalConfig.emergency_pause`).
- Reentrancy lock (`GlobalConfig.reentrancy_lock`) around CPI paths.
- Oracle signature verification via Solana Ed25519 program + instruction sysvar inspection.
- Nullifier (SHA-256 hash of oracle payload) to prevent replay.
- TWAP-based circuit breaker using Pyth account data checks.

---

## Off-Chain Components

### 1) Telegram Bot (`bot`)

TypeScript bot using `grammy`, Solana RPC helpers, and Privy server auth.

### User Commands
- `/start` — create/retrieve embedded wallet and provide initialize-user link.
- `/credit` — show reputation and max borrowable amount.
- `/verify` — deep-link to verification flow.
- `/withdraw` — generate/share loan Blink URL.

### Notable Behavior
- Hindi-first user messaging.
- RPC calls with timeout and friendly retry handling.
- Mirrors on-chain max-loan logic client-side for display.

### 2) Blink / Solana Actions (`blink`)

TypeScript module for Solana Actions-compatible loan interactions:
- GET metadata and max borrowable display.
- POST creates unsigned `VersionedTransaction` for `request_loan`.
- User signs client-side wallet transaction; server does not hold private keys.

---

## Configuration

### Shared / Protocol Variables
- `KOSH_PROGRAM_ID` — deployed program ID (used by bot/blink; defaults are placeholders).
- `HELIUS_RPC_URL` — RPC endpoint (defaults to Solana devnet RPC in code if unset).
- `SOL_TO_INR` — display-only conversion approximation.

### Bot-Specific
- `BOT_TOKEN` (**required**)
- `BLINK_BASE_URL`
- `TWA_VERIFY_URL`
- `PRIVY_APP_ID`
- `PRIVY_APP_SECRET`

### Blink-Specific
- `PYTH_SOL_USD_PRICE_ACCOUNT` (for loan instruction account wiring)

---

## Development Setup

### Prerequisites
- Rust toolchain (compatible with Anchor/Solana stack)
- Solana CLI
- Anchor CLI (`0.30.1` expected by `Anchor.toml`)
- Node.js + npm

### Install Dependencies

```bash
# repo root
cd KOSH.id

# Rust deps are fetched automatically by cargo commands.

# TypeScript deps
cd bot && npm ci
cd ../blink && npm ci
```

---

## Build, Type-Check, and Test

From repository root:

```bash
# Rust unit tests (program utilities and related tests)
cargo test --package kosh --lib

# Bot type-check
cd bot && npx tsc --noEmit

# Blink type-check
cd ../blink && npx tsc --noEmit
```

Optional TypeScript build/start:

```bash
cd bot && npm run build && npm run start
cd ../blink && npm run build && npm run start
```

---

## Operational Notes

- This repository contains v1 logic and explicit TODOs for future hardening/features (e.g., full zk-proof verification, yield integrations).
- Some default IDs in code are placeholders; set production values via environment configuration before deployment.
- Bot and Blink currently rely on runtime integration (web framework/server routing) outside this repository’s minimal module code.

---

## License

This project is licensed under the terms in [`LICENSE`](./LICENSE).
