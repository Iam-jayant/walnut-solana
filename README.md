# Walnut Protocol 🌰

> **Confidential Lending & Liquidity on Solana L1, powered by Arcium MPC.**

[![Solana Devnet](https://img.shields.io/badge/Solana-Devnet-14F195?logo=solana&logoColor=black)](https://explorer.solana.com/address/5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea?cluster=devnet)
[![Arcium MPC](https://img.shields.io/badge/Arcium_MPC-v0.16.0-FFD208?logoColor=black)](https://arcium.com)
[![Anchor](https://img.shields.io/badge/Anchor-v1.0.2-black)](https://anchor-lang.com)
[![Next.js](https://img.shields.io/badge/Next.js-16_Turbopack-black?logo=next.js)](https://nextjs.org)
[![TypeScript](https://img.shields.io/badge/TypeScript-5.9-blue?logo=typescript)](https://www.typescriptlang.org)

---

## Overview

In traditional public DeFi (Aave, Kamino, Marginfi), every wallet's collateral, debt, borrow limits, and liquidation prices are visible to anyone on Solscan. This transparency invites **predatory MEV bots, front-running, and liquidation cascades**, while preventing institutions and funds from executing confidential lending strategies.

**Walnut closes this gap.** Built natively on Solana L1 and integrated with the **Arcium Multi-Party Computation (MPC)** network, Walnut encrypts borrower balances, debt, and collateral onchain. Mathematical rules—such as loan-to-value (LTV) limits, continuous interest accrual, and solvency checks—are verified privately in MPC enclaves without publishing raw financial data to the public ledger.

```
       [ Public Solana L1 ]                           [ Arcium MPC Enclave ]
+--------------------------------+             +----------------------------------+
|  User Deposits SOL / USDC      |             |                                  |
|               │                |             |  Computes Encrypted Deltas       |
|               ▼                |             |  Verifies LTV & Health Ratios    |
|  Wrapped into cSOL & cUSDC     | ──────────> |  Zero-Knowledge Solvency Check   |
|  (Balances Encrypted On-Chain) |             |  Accrues Confidential Interest   |
|               │                |             |                                  |
|               ▼                | <────────── |                                  |
|  Encrypted State Committed     |             +----------------------------------+
+--------------------------------+
```

---

## Key Features

- **Confidential Balances (`cSOL` & `cUSDC`)**: Wrap native SOL and USDC into confidential token balances where ciphertext balances are stored directly on Solana L1.
- **Private Collateral & Borrowing**: Post encrypted SOL collateral and borrow USDC. Accrued interest updates continuously without public balance leakages.
- **Confidential Yield (Supply & Earn)**: Supply USDC to the lending pool to earn real-time APY. Pool shares and earned interest remain private.
- **Zero-Knowledge Health Checks & Liquidations**: Anyone can trigger permissionless health checks for any borrower. The Arcium MPC returns a single boolean outcome (`Healthy` vs `Below Liquidation Threshold`) without revealing underlying debt or collateral amounts.
- **Client-Side Viewing Key Custody**: Users sign a zero-gas message to derive an in-memory Viewing Key. Keys feature automatic 5-minute inactivity timeouts to protect against shoulder surfing and open tab exploits.
- **Selective Auditor Compliance**: Disclose specific balance fields (e.g. proof of assets, tax compliance) to authorized auditor public keys using x25519 asymmetric encryption.
- **Zama-Inspired Chamfered UI**: Production web application built with chamfered cards (`rounded-2xl`), ambient frosted glassmorphism, real-time toast alerts, and direct Solana Explorer links.

---

## Deployed Onchain Contracts (Solana Devnet)

Walnut is live and initialized on Solana Devnet:

| Contract / Account | Address |
| :--- | :--- |
| **Walnut Program ID** | [`5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea`](https://explorer.solana.com/address/5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea?cluster=devnet) |
| **Lending Market Address** | [`Du5ZWegZJj1RvpZ2GHVNgsy2MXAhiThpP3AHWRkJEEGr`](https://explorer.solana.com/address/Du5ZWegZJj1RvpZ2GHVNgsy2MXAhiThpP3AHWRkJEEGr?cluster=devnet) |
| **Confidential USDC Mint** | [`GHLMq3gCUHkb9sGPouXNo4Lt9QpGCmz5UMrtF7EwVDLT`](https://explorer.solana.com/address/GHLMq3gCUHkb9sGPouXNo4Lt9QpGCmz5UMrtF7EwVDLT?cluster=devnet) |
| **Wrapped SOL Mint** | [`So11111111111111111111111111111111111111112`](https://explorer.solana.com/address/So11111111111111111111111111111111111111112?cluster=devnet) |
| **Arcium Cluster Offset** | `456` (Shared Devnet Cluster) |

---

## Repository Architecture

```
walnut-solana/
├── programs/walnut/           # Anchor Solana on-chain smart contract
├── encrypted-ixs/             # Arcis Rust circuits compiled for Arcium MPC
├── packages/sdk/              # @walnut/sdk TypeScript client library
├── packages/reference-model/  # High-precision simulation model for verification
├── web/                       # Next.js 16 Web Application (App Router, Tailwind CSS)
│   ├── app/                   # Next.js routes (/app, /app/audit, /api/band/refresh)
│   ├── components/            # Zama-styled UI components, toast provider, sidebar
│   └── lib/                   # Solana web3 RPC, Anchor IDL, arithmetic & metrics
├── keeper/                    # Background rate curve & band keeper service
├── scripts/                   # Toolchain validation, deployment scripts & test suites
└── tests/                     # Integration tests across Anchor & Arcium
```

---

## Deploying to Vercel

The web application is fully optimized for **Vercel** with instant Turbopack builds and automated cron-based band refresh.

### Step 1: Import Repository
1. Log in to [Vercel](https://vercel.com) and click **"Add New Project"**.
2. Select your `walnut-solana` repository.

### Step 2: Configure Project Settings
- **Framework Preset**: `Next.js`
- **Root Directory**: Click "Edit" and select **`web`** (ensuring files outside root directory remain accessible for `@walnut/sdk`).
- **Build Command**: `pnpm sdk:build && next build` (or leave default `npm run build`)
- **Output Directory**: `.next`

### Step 3: Environment Variables
Configure the following variable in Vercel project settings:

| Name | Value | Description |
| :--- | :--- | :--- |
| `NEXT_PUBLIC_RPC_URL` | `https://api.devnet.solana.com` | Solana Devnet RPC endpoint (or custom Helius/QuickNode RPC) |

### Step 4: Deploy
Click **Deploy**. Your confidential lending application will be live in under 2 minutes!

---

## Local Development Setup

### Prerequisites
- **Node.js**: `>= 20.18.0`
- **pnpm**: `>= 9.15.0`
- **Rust & Solana CLI** *(optional, only if compiling onchain programs)*

### 1. Clone & Install Dependencies
```bash
git clone https://github.com/Iam-jayant/walnut-solana.git
cd walnut-solana
pnpm install
```

### 2. Build the TypeScript SDK
```bash
pnpm --filter @walnut/sdk build
```

### 3. Run the Web dApp Locally
```bash
cd web
pnpm dev
```
Open [http://localhost:3000](http://localhost:3000) in your browser. Connect any Solana wallet (Phantom, Solflare, Backpack) on **Devnet**.

---

## Protocol Lifecycle

1. **Shield (`Deposit`)**: Wraps public SOL or USDC into encrypted `cSOL` or `cUSDC`.
2. **Post Collateral**: Allocates confidential SOL to back your personal borrow limit.
3. **Borrow USDC**: Computes available liquidity and mints confidential USDC up to 50% LTV.
4. **Supply USDC**: Lends confidential USDC into the shared liquidity pool, earning dynamic interest rates.
5. **Repay & Unshield**: Repays outstanding debt and burns/unwraps confidential tokens back to public Solana assets.
6. **Liquidate**: Permissionless solvency check runs inside Arcium MPC to protect pool health with a 5% collateral bonus for liquidators.

---

## Security & Verification

- **State Encryption**: Ciphertexts use Rescue / ElGamal encryption verified inside Arcium MPC nodes.
- **Position Locks**: Double-spend prevention via optimistic account locks on Solana L1 while multi-party computations finalize.
- **Non-Custodial Viewing Keys**: Zero plaintext balance information is stored on backend servers or databases. Decryption is performed purely in client memory.

---

## License

This project is licensed under the Apache 2.0 / MIT License.
