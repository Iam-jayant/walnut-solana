# Walnut Protocol — Comprehensive Audit Report

**Date:** 2026-11-25  
**Auditor:** Professional Software Architect Review  
**Scope:** Full codebase audit, mainnet readiness assessment, token requirements, treasury funding, revenue model, and security posture  

---

## Table of Contents

1. [Project Understanding](#1-project-understanding)
2. [Architecture Overview](#2-architecture-overview)
3. [Implementation Completeness](#3-implementation-completeness)
4. [Mainnet Readiness Assessment](#4-mainnet-readiness-assessment)
5. [Token Requirements for Mainnet](#5-token-requirements-for-mainnet)
6. [Treasury Funding Requirements](#6-treasury-funding-requirements)
7. [Revenue Model Analysis](#7-revenue-model-analysis)
8. [Security Findings](#8-security-findings)
9. [DDoS Protection & Vercel Hardening](#9-ddos-protection--vercel-hardening)
10. [Mainnet Migration Checklist](#10-mainnet-migration-checklist)
11. [Recommendations Summary](#11-recommendations-summary)

---

## 1. Project Understanding

### What Walnut Is
Walnut is a **confidential lending and liquidity protocol on Solana L1** powered by **Arcium MPC (Multi-Party Computation)**. It is the first protocol to offer truly private DeFi lending — user balances, collateral, and debt are encrypted on-chain, and only the account owner can decrypt them using a Viewing Key derived from their wallet signature.

### Core Value Proposition
- **Privacy-preserving lending:** SOL collateral → borrow USDC, with all position balances (free SOL, collateral SOL, free USDC, debt shares, supply shares) stored as encrypted ciphertexts
- **Confidential liquidation:** Health checks and liquidations happen via MPC without revealing position details publicly
- **Interest-bearing supply:** Users can supply USDC to earn yield, with interest accrual tracked via supply/borrow index model
- **Utilization-band privacy (ADR-0004):** Instead of public totals, the protocol reveals only which utilization band (5% granularity) the pool falls into, preserving aggregate privacy

### Technology Stack
| Layer | Technology | Version |
|-------|-----------|---------|
| Smart Contract | Anchor (Solana) | 1.0.2 |
| Solana CLI | solana-cli | 3.1.10 |
| MPC Framework | Arcium | 0.16.0 |
| Frontend | Next.js 16 + Turbopack | Latest |
| Styling | Tailwind CSS | 4.x |
| Package Manager | pnpm monorepo | Workspaces |
| Oracle | Pyth Network | SOL/USD feed |
| Wallet | Solana Wallet Adapter | Standard |

---

## 2. Architecture Overview

### Monorepo Structure
```
walnut-solana/
├── programs/walnut/         # Anchor smart contract (lib.rs ~316KB)
├── encrypted-ixs/           # Arcium MPC circuits (lib.rs ~85KB)
├── packages/
│   ├── sdk/                 # TypeScript SDK (20 source files)
│   └── reference-model/     # Reference model for property tests
├── web/                     # Next.js 16 frontend
├── keeper/                  # Autonomous keeper service (stub)
├── scripts/                 # Deploy scripts (lend-devnet.sh)
├── config/networks/         # Network configs (devnet.json only)
├── docs/                    # Comprehensive documentation (15+ docs)
└── .github/workflows/       # CI pipeline (ci.yml)
```

### Smart Contract Modules
The program (`5EunnixqNofjZDMwZudttJCKq675d99dwjqR8jTcjea`) contains:
- **Shield/Unshield** — Deposit/withdraw SOL and USDC to/from encrypted positions
- **Lend Operations** — AddCollateral, WithdrawCollateral, Borrow, Repay, Supply, WithdrawSupply
- **Health Check & Liquidation** — Permissionless health checks, confidential liquidation via MPC
- **Pool Operations** — Encrypted pool state management (ADR-0004)
- **Band Rate Source** — Utilization band-based interest accrual for privacy
- **Disclosure** — Selective position disclosure to authorized parties
- **Key Rotation** — Viewing key rotation for position security
- **Recovery** — Empty pool recovery mechanism
- **Governance** — Timelocked config changes (24hr–14day delay), role management, pause controls
- **Reserve Withdrawal** — Protocol treasury operations with timelock

### MPC Circuits (Arcium)
7 active circuits for mainnet:
1. `lend_op` — Core lending operations (collateral, borrow, repay, supply, health, liquidation)
2. `disclose_position` — Selective position disclosure
3. `rotate_position_key` — Viewing key rotation
4. `pool_op` — Encrypted pool state operations
5. `position_private` — Private collateral operations
6. `liquidate_pool` — Pool-level liquidation
7. `pool_band` — Utilization band updates

Legacy circuits to reclaim (saves rent):
- `add_together`, `init_position`, `apply_position_delta`, `liquidation_receipts`, `oversized_output`, `rotate_viewing_key`

### Interest Rate Model
- **Kinked model:** base rate + slope1 (below optimal utilization) + slope2 (above optimal)
- **Default params:** base=0%, slope1=4%, slope2=75%, optimal=80%
- **Max borrow rate:** 1,000% APR (100,000 bps) safety cap
- **Reserve factor:** Splits borrower interest between suppliers and protocol reserves
- **1e9-scale indexes:** `borrow_index_1e9` and `supply_index_1e9` for continuous accrual
- **Band-based accrual (ADR-0004):** Conservative upper-edge rate for borrowers, lower-edge for suppliers

---

## 3. Implementation Completeness

### ✅ Complete and Working

| Feature | Status | Evidence |
|---------|--------|----------|
| Core smart contract | ✅ Complete | All instruction modules present, 316KB lib.rs |
| MPC circuits (7 active) | ✅ Complete | 85KB encrypted-ixs/lib.rs, all compiled |
| Shield/Unshield (SOL & USDC) | ✅ Complete | Full instructions in program |
| Lending operations (6 ops) | ✅ Complete | lend_op circuit + SDK mirror in lend-op.ts |
| Health check & liquidation | ✅ Complete | Permissionless, confidential via MPC |
| Private operations (ADR-0004) | ✅ Complete | pool_op, position_private, pool_band circuits |
| Supply/withdraw supply | ✅ Complete | supply_flow in program + SDK |
| Interest accrual (indexes) | ✅ Complete | Both totals and banded accrual, property tested |
| Utilization bands (20 × 5%) | ✅ Complete | Band rate source, daily refresh via Vercel cron |
| Viewing key derivation | ✅ Complete | Ed25519 → x25519 in viewing-key.ts (15KB) |
| Position encryption/decryption | ✅ Complete | Full encrypted state management |
| Pyth oracle integration | ✅ Complete | SOL/USD with sanity clamping, staleness checks |
| Timelocked governance | ✅ Complete | 24hr–14day delays, cross-bound validation |
| Role management | ✅ Complete | Admin, keeper, operator roles |
| Pause controls | ✅ Complete | Per-operation pause capability |
| Position/debt caps | ✅ Complete | Configurable per market |
| Reserve withdrawal | ✅ Complete | Timelocked treasury operations |
| TypeScript SDK | ✅ Complete | 20 source files, full mirror of on-chain logic |
| Web frontend | ✅ Complete | Dashboard, action panel, liquidations, landing page |
| Deploy script (devnet) | ✅ Complete | Resumable 8-step lend-devnet.sh (15KB) |
| CI pipeline | ✅ Complete | 4-job CI: JS, Rust+Arcium, audit, secrets scan |
| Documentation | ✅ Complete | Architecture, privacy model, threat model, ADRs, demos |
| Property tests | ✅ Complete | Rust + TypeScript property tests for math |
| Secret scanning (gitleaks) | ✅ Complete | Full git history scan in CI |

### ⚠️ Incomplete / Needs Work

| Feature | Status | Detail |
|---------|--------|--------|
| Mainnet network config | ❌ Missing | Only `config/networks/devnet.json` exists |
| Mainnet deploy script | ❌ Missing | `lend-devnet.sh` is devnet-specific; needs mainnet variant |
| Keeper service | ⚠️ Stub | `keeper/package.json` exists but no runtime code |
| Revenue model (1% fee) | ⚠️ Partial | Reserve factor exists but no transaction fee |
| Web config for mainnet | ❌ Hardcoded | `web/lib/config.ts` all devnet values |
| Environment variable management | ❌ Insecure | API key exposed, private key hardcoded |
| Explorer URL | ❌ Hardcoded | `?cluster=devnet` in explorerTx() |
| DDoS protection | ❌ Not configured | No Vercel WAF/bot protection settings |
| Formal audit (security) | ❌ Not done | No third-party smart contract audit |

### 🗑️ Dead Code / Feasibility Scaffolding
The following comp def offsets are from early feasibility spikes and are still registered on devnet:
- `add_together` — Feasibility test circuit
- `init_position` — Old position initialization
- `apply_position_delta` — Old delta application
- `liquidation_receipts` — Replaced by integrated liquidation
- `oversized_output` — Output size test
- `rotate_viewing_key` — Replaced by `rotate_position_key`

The deploy script already handles reclaiming these (step 2), recovering their rent.

---

## 4. Mainnet Readiness Assessment

### Smart Contract: 🟡 READY WITH CHANGES

**What's ready:**
- Core logic is complete and tested
- Property tests cover critical math paths (index monotonicity, interest conservation)
- Transaction size tests ensure instructions fit block limits
- The expanded mainnet block size (4000+ bytes vs 1200 devnet) gives comfortable headroom — no instruction restructuring needed

**What needs changing before deploy:**
1. **Anchor.toml:** Change `cluster = "localnet"` → `cluster = "mainnet-beta"`
2. **Program ID:** Generate a new mainnet keypair or reuse the devnet one (see token requirements)
3. **Config values:** New market initialization with mainnet USDC mint (`EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`), mainnet wSOL, mainnet Pyth oracle
4. **Deploy script:** Create `lend-mainnet.sh` adapted from `lend-devnet.sh`
5. **Initial parameters:** Review risk params (LTV, liquidation threshold, caps) for mainnet launch

### Application Infrastructure: 🟡 READY WITH CHANGES

**What's ready:**
- Next.js 16 app with full UI (dashboard, action panel, liquidations, landing page)
- Local fonts (Telegraf) — no external font requests (CSP compliant)
- Turbopack build with proper shims for Arcium browser compatibility
- Wallet Adapter integration

**What needs changing:**
1. **`web/lib/config.ts`:** All constants must switch to mainnet values
2. **Environment variables:** RPC URL, API keys must be server-side env vars, not `NEXT_PUBLIC_`
3. **Explorer URL:** Remove `?cluster=devnet` hardcoding
4. **Band refresh cron:** Update keeper keypair to mainnet (remove hardcoded bytes)
5. **Vercel deployment:** Enable WAF, bot protection, rate limiting

---

## 5. Token Requirements for Mainnet

### Methodology
Costs calculated using:
- Rent-exempt rate: **6,960 lamports per byte** (6.96×10⁻⁶ SOL/byte, 2-year minimum)
- Current SOL price assumption: **~$150** (the codebase uses FALLBACK_SOL_PRICE = 150,000,000 for 1e6 units)
- Priority fees: 50,000 micro-lamports per CU (as used in deploy script)
- Note: program deploy buffer rent is **refunded** after successful deploy

### A. Program Deployment

| Item | Size/Count | SOL Cost | Notes |
|------|-----------|----------|-------|
| Program binary (walnut.so) | ~300–400 KB | ~2.8–3.8 SOL | Rent for program data account; buffer rent refunded after deploy |
| Program extend (if reusing ID) | Variable | ~0.14 SOL per 20KB | Only if mainnet binary > current on-chain allocation |
| **Subtotal (program)** | | **~3–4 SOL** | |

### B. Circuit Uploads (7 circuits)

| Circuit | Estimated Size | SOL Cost |
|---------|---------------|----------|
| lend_op | ~50–100 KB | ~0.35–0.70 |
| disclose_position | ~30–60 KB | ~0.21–0.42 |
| rotate_position_key | ~20–40 KB | ~0.14–0.28 |
| pool_op | ~50–100 KB | ~0.35–0.70 |
| position_private | ~40–80 KB | ~0.28–0.56 |
| liquidate_pool | ~40–80 KB | ~0.28–0.56 |
| pool_band | ~20–40 KB | ~0.14–0.28 |
| **Subtotal (circuits)** | | **~2–4 SOL** | Comp def registration + raw circuit accounts |

### C. Account Creation (Rent-Exempt)

| Account | Size (bytes) | SOL Cost |
|---------|-------------|----------|
| Market | 668 | 0.0046 |
| AssetState | 319 | 0.0022 |
| LendingPool | 253 | 0.0018 |
| wSOL Vault (token account) | 165 | 0.0012 |
| USDC Vault (token account) | 165 | 0.0012 |
| Treasury USDC Account | 165 | 0.0012 |
| Vault Authority (PDA) | 0 | 0 (PDA, no rent) |
| Operation Counters (per user) | ~40 | Paid by users |
| Positions (per user) | ~500+ | Paid by users |
| **Subtotal (accounts)** | | **~0.015 SOL** |

### D. Transaction Fees (Deploy Process)

| Step | Estimated Transactions | SOL Cost |
|------|----------------------|----------|
| Program write-buffer | ~50–100 chunks | ~0.03 |
| Program deploy | 1 | ~0.005 |
| Circuit uploads (7 × multi-chunk) | ~200–400 chunks | ~0.1 |
| Market + pool initialization | ~5–10 txns | ~0.005 |
| Band setup + initial observation | ~3–5 txns | ~0.003 |
| Reclaim old comp defs (devnet rent) | ~12–20 txns | ~0.01 |
| **Subtotal (tx fees)** | | **~0.15 SOL** |

### E. Operational Reserve

| Item | SOL Cost | Notes |
|------|----------|-------|
| Deployer reserve (kept free) | 1.0 | Script enforces 1 SOL minimum reserve |
| Keeper operations (band refresh) | 0.5 | ~365 daily txns/year at ~0.001 SOL each |
| Priority fee buffer | 0.5 | Congestion periods on mainnet |
| **Subtotal (operations)** | **2.0 SOL** | |

### TOTAL TOKEN REQUIREMENTS

| Category | SOL | USD (at $150) |
|----------|-----|---------------|
| Program deployment | 3–4 | $450–600 |
| Circuit uploads | 2–4 | $300–600 |
| Account creation | 0.015 | $2.25 |
| Transaction fees | 0.15 | $22.50 |
| Operational reserve | 2.0 | $300 |
| **TOTAL (conservative)** | **~8–10 SOL** | **~$1,200–1,500** |
| **TOTAL (optimistic, with reclaimed devnet rent)** | **~6–8 SOL** | **~$900–1,200** |

> **Note on expanded block size (4000+ bytes):** The mainnet block size increase means no instruction splitting is needed. All existing transactions fit comfortably. This is purely a positive — it may reduce the number of transactions needed for some operations, slightly lowering fee costs.

> **Cost optimization:** The deploy script already implements rent reclamation from unused comp defs (step 2). On mainnet, reclaiming the 6 legacy circuits could recover ~1–2 SOL.

---

## 6. Treasury Funding Requirements

### Existing Treasury Infrastructure
The smart contract has built-in treasury support:
- `treasury_authority` (Pubkey) at Market offset 140 — the authority that can withdraw reserves
- `treasury_usdc_account` (Pubkey) at Market offset 460 — receives withdrawn reserves
- `reserves_usdc` (u64) in LendingPool — protocol reserves accumulated from interest
- `set_treasury_usdc_account` — admin instruction to configure the USDC receiving account
- `reserve_withdrawal` — timelocked instruction to withdraw accumulated reserves

### Required Treasury Funding

| Treasury | Required Balance | Purpose |
|----------|-----------------|---------|
| Treasury USDC Account | ≥ $10 USDC | Minimum viable balance as specified. Must be a token account owned by `treasury_authority` |
| wSOL Vault | Funded by user deposits | No pre-funding needed |
| USDC Vault | Funded by user deposits | No pre-funding needed |
| Deployer wallet | ~10 SOL | Program deploy + circuits + operations |

### Action Items
1. **Create a mainnet treasury USDC token account** owned by the treasury authority multisig/wallet
2. **Fund it with ≥ $10 USDC** (10,000,000 USDC base units since USDC has 6 decimals)
3. **Call `set_treasury_usdc_account`** after market initialization to register it
4. **The keeper wallet** needs ~0.5 SOL for band refresh operations (daily Vercel cron)

### Treasury Balance Verification
Currently on devnet, the treasury setup should be verified:
- Check that `treasury_authority` is set to the correct admin pubkey
- Check that `treasury_usdc_account` points to a valid, funded USDC token account
- Confirm reserves_usdc in LendingPool is accumulating (proves the reserve factor works)

---

## 7. Revenue Model Analysis

### Current Revenue Model: Reserve Factor (Exists)
The protocol **already has a revenue model** via the **reserve factor**:

```
Interest paid by borrowers → split:
  ├── (1 - reserve_factor) → goes to suppliers (supply index growth)
  └── reserve_factor       → goes to reserves_usdc (protocol revenue)
```

- **Implementation:** `LendingPool::accrue()` in `lending_pool.rs` — `reserve_credit = interest * reserve_factor_bps / 10000`
- **Accumulation:** `reserves_usdc: u64` in LendingPool state
- **Withdrawal:** `reserve_withdrawal` instruction with timelock protection
- **Configurable:** `reserve_factor_bps` in AssetState risk params, changeable via timelocked governance

**This is a standard DeFi revenue model** (Aave, Compound use the same approach). Revenue grows with TVL and borrowing activity.

### Requested: 1% Transaction Fee

The user requested implementing a **1% fee that goes to treasury** if no revenue model exists. Since a reserve factor already exists, the 1% fee would be **additional** revenue.

#### Implementation Options for 1% Fee

**Option A: Increase the Reserve Factor (Recommended — Zero Code Changes)**
- Set `reserve_factor_bps` to a value that effectively captures ~1% of all lending volume
- Example: If average utilization is ~50%, a 20% reserve factor captures ~10% of interest, which could approximate 1% of volume
- **Pro:** No smart contract changes, configurable via governance
- **Con:** Revenue depends on borrowing activity, not raw transaction volume

**Option B: Add an Explicit 1% Transaction Fee (Requires Contract Changes)**
- Add a fee deduction in `shield` (deposit) or `unshield` (withdraw) instructions
- On every deposit: 1% of deposited amount goes to treasury
- **Pro:** Predictable fee, independent of borrowing
- **Con:** Requires smart contract modification, redeployment, and re-audit. Deposit fees are user-hostile and may deter adoption. Unusual for lending protocols.

**Option C: Add a 1% Fee on Borrow Origination (Moderate Code Changes)**
- When a user borrows USDC, 1% of the borrowed amount is charged upfront
- Added as extra debt shares or deducted from disbursed amount
- **Pro:** Standard in some protocols (origination fee)
- **Con:** Requires circuit modification (MPC circuit changes are complex), program changes, and testing

#### Recommendation
**Use Option A (increase reserve factor) for launch.** It requires zero code changes and can be tuned via governance. A 10–20% reserve factor is industry standard. If additional fee revenue is needed post-launch, Option C (borrow origination fee) can be added in a future upgrade.

---

## 8. Security Findings

### 🔴 CRITICAL

#### 8.1 Hardcoded Private Key in Source Code
**File:** `web/app/api/band/refresh/route.ts`
```
const DEVNET_KEEPER_BYTES = [100, 186, 244, 1, 137, 116, ...];
```
- The keeper keypair (pubkey: `9FV3uQ7bEQgyMMt4rs6Hj23Kd6JCzE3hsQwiU2CFLPsK`) is hardcoded as a byte array
- **Risk:** Anyone with repo access has the keeper's private key
- **Impact:** On mainnet, this would allow unauthorized band refresh operations
- **Fix:** Move to server-side environment variable (`KEEPER_PRIVATE_KEY` in Vercel)

#### 8.2 Exposed Helius API Key
**File:** `web/.env.local`
```
NEXT_PUBLIC_RPC_URL=https://devnet.helius-rpc.com/?api-key=24db9681-...
```
- The API key is committed to the repository AND exposed as `NEXT_PUBLIC_` (embedded in browser bundle)
- **Risk:** Key abuse, rate limit exhaustion, potential billing impact
- **Impact:** Anyone can use this API key; it appears in every user's browser network tab
- **Fix:** 
  1. Rotate the Helius API key immediately
  2. Use server-side proxy for RPC calls (API route that forwards to Helius)
  3. Never use `NEXT_PUBLIC_` for API keys

### 🟠 HIGH

#### 8.3 All Configuration Hardcoded to Devnet
**File:** `web/lib/config.ts`
- `NETWORK`, `PROGRAM_ID`, `MARKET_ADDRESS`, `USDC_MINT`, `CLUSTER_OFFSET`, `FALLBACK_SOL_PRICE` — all devnet values
- Explorer URL function returns `?cluster=devnet`
- **Risk:** Switching to mainnet requires touching a core config file; easy to miss values
- **Fix:** Environment-driven configuration with separate devnet/mainnet profiles

#### 8.4 No Mainnet Network Configuration
**File:** `config/networks/devnet.json` (exists) vs `mainnet.json` (missing)
- The SDK's `network-profile.ts` (21KB) presumably reads from these configs
- No mainnet profile means no mainnet USDC mint, no mainnet vaults, no mainnet oracle addresses
- **Fix:** Create `config/networks/mainnet.json` with all mainnet addresses

#### 8.5 Gitleaks May Not Catch the Hardcoded Key
- The CI runs `gitleaks` for secret scanning, which is good
- However, the keeper key is a raw byte array, not a standard secret format
- Gitleaks patterns may not flag it
- **Fix:** Add a custom gitleaks rule or remove the hardcoded key entirely

### 🟡 MEDIUM

#### 8.6 Keeper Service is a Stub
**File:** `keeper/package.json`
- Only build/typecheck scripts, no runtime code
- The band refresh currently runs as a Vercel cron (`/api/band/refresh`) with the hardcoded keeper key
- **Risk:** Single point of failure; if Vercel cron fails, utilization bands go stale
- **Fix:** For mainnet, consider a proper keeper service (could be a simple cron job on a VPS) or keep the Vercel cron but with proper key management

#### 8.7 No Rate Limiting on API Routes
- `web/app/api/band/refresh/route.ts` is a public API endpoint
- No authentication beyond the internal keeper key check
- **Risk:** Could be called repeatedly, wasting keeper SOL on transaction fees
- **Fix:** Add Vercel WAF rules or API route protection

#### 8.8 USDC Mint is Devnet Faucet Token
- Current USDC mint: `GHLMq3gCUHkb9sGPouXNo4Lt9QpGCmz5UMrtF7EwVDLT` (devnet faucet)
- Mainnet USDC: `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`
- **Risk:** If not changed, mainnet would use a worthless devnet token
- **Fix:** Included in mainnet config migration

### 🟢 LOW / INFORMATIONAL

#### 8.9 No Third-Party Smart Contract Audit
- The protocol handles real user funds on mainnet
- MPC circuits add additional complexity that standard auditors may not cover
- **Recommendation:** Budget for a professional audit before significant TVL

#### 8.10 `suppressHydrationWarning` in Layout
- Used in `web/app/layout.tsx` on `<html>` and `<body>`
- Likely needed for theming/dark mode, but masks genuine hydration issues
- **Risk:** Low; standard Next.js pattern

#### 8.11 CI Does Not Include Mainnet Deploy Verification
- The CI pipeline is comprehensive (format, lint, types, unit tests, property tests, Arcium localnet tests, dependency audit, secret scan)
- No mainnet deployment verification or staging environment tests
- **Recommendation:** Add a staging deploy step for pre-mainnet validation

---

## 9. DDoS Protection & Vercel Hardening

### Current State
- No DDoS protection configured
- No bot management enabled
- No WAF (Web Application Firewall) rules
- The Vercel cron endpoint is unprotected

### Recommended Vercel Configuration

#### 9.1 Enable Vercel Firewall (WAF)
In Vercel Dashboard → Project → Security:
- **Enable Firewall:** Turn on the Vercel WAF
- **Challenge mode:** Enable for suspicious traffic
- **IP blocking:** Block known malicious IP ranges

#### 9.2 Rate Limiting
Add rate limiting rules:
- `/api/*` endpoints: Max 60 requests/minute per IP
- `/api/band/refresh`: Max 2 requests/day (matches cron schedule)
- General page requests: Max 100 requests/minute per IP

#### 9.3 Bot Protection
- **Enable Bot Protection:** Vercel's built-in bot detection
- **Challenge suspicious bots:** CAPTCHA or JS challenge for suspected bots
- **Allow legitimate bots:** Google, Bing crawlers (for the landing page SEO)

#### 9.4 Headers & CSP
Add to `vercel.json` or `next.config.ts`:
```json
{
  "headers": [
    {
      "source": "/(.*)",
      "headers": [
        { "key": "X-Frame-Options", "value": "DENY" },
        { "key": "X-Content-Type-Options", "value": "nosniff" },
        { "key": "Referrer-Policy", "value": "strict-origin-when-cross-origin" },
        { "key": "Permissions-Policy", "value": "camera=(), microphone=(), geolocation=()" }
      ]
    }
  ]
}
```
- The app already loads fonts locally (strict CSP compliant per Requirement 14)
- Add CSP headers restricting script/connect sources to self + RPC endpoints

#### 9.5 Protect the Cron Endpoint
- Use Vercel's `CRON_SECRET` environment variable to authenticate cron requests
- Verify `Authorization: Bearer ${CRON_SECRET}` header in the route handler

---

## 10. Mainnet Migration Checklist

### Phase 1: Pre-Deploy Preparation
- [ ] **Rotate the exposed Helius API key** — do this immediately regardless of mainnet timeline
- [ ] **Remove hardcoded keeper private key** from source code; move to env var
- [ ] **Create `config/networks/mainnet.json`** with:
  - Mainnet program ID (new keypair or reused)
  - Mainnet USDC mint: `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`
  - Mainnet wSOL: `So11111111111111111111111111111111111111112` (same on all networks)
  - Mainnet Pyth oracle address for SOL/USD
  - Appropriate cluster offset for mainnet Arcium
- [ ] **Create `lend-mainnet.sh`** adapted from `lend-devnet.sh`
- [ ] **Update `web/lib/config.ts`** to be environment-driven
- [ ] **Fix explorer URL** to omit `?cluster=devnet` on mainnet
- [ ] **Set up Vercel environment variables** for mainnet:
  - `RPC_URL` (server-side only, no `NEXT_PUBLIC_`)
  - `KEEPER_PRIVATE_KEY` (server-side only)
  - `CRON_SECRET` (for cron authentication)
  - `NEXT_PUBLIC_NETWORK` (to switch config profiles)
- [ ] **Review and set mainnet risk parameters:**
  - LTV ratio (conservative for launch)
  - Liquidation threshold
  - Position/debt caps
  - Reserve factor (revenue model)
  - Timelock delay (recommend 48h+ for mainnet)

### Phase 2: Deploy
- [ ] **Fund deployer wallet** with ~10 SOL
- [ ] **Build program and circuits** (`pnpm run build`)
- [ ] **Deploy program** to mainnet via adapted deploy script
- [ ] **Upload all 7 circuits** to mainnet Arcium cluster
- [ ] **Initialize market** with mainnet parameters
- [ ] **Initialize LendingPool** 
- [ ] **Set up band rate source** and initial observation
- [ ] **Create and fund treasury USDC account** (≥ $10 USDC)
- [ ] **Register treasury** via `set_treasury_usdc_account`
- [ ] **Fund keeper wallet** (~0.5 SOL for band refresh operations)

### Phase 3: Verify
- [ ] **Verify program on-chain** matches local build (deploy script step 8)
- [ ] **Test shield/unshield** with small amounts
- [ ] **Test lend operations** (collateral, borrow, repay, supply)
- [ ] **Test band refresh** via manual cron trigger
- [ ] **Verify treasury accumulation** — confirm reserves_usdc increments
- [ ] **Test liquidation flow** with a test position
- [ ] **Verify explorer links** point to mainnet transactions

### Phase 4: Harden
- [ ] **Enable Vercel WAF** and bot protection
- [ ] **Configure rate limiting** on API routes
- [ ] **Add security headers** (CSP, X-Frame-Options, etc.)
- [ ] **Set up monitoring/alerting** for:
  - Keeper cron failures
  - Oracle staleness
  - Unusual transaction patterns
  - Deployer/keeper balance running low
- [ ] **Document emergency procedures** (pause, role transfer)

### Phase 5: Post-Launch (Recommended)
- [ ] **Professional smart contract audit** before significant TVL
- [ ] **Build proper keeper service** to replace Vercel cron stub
- [ ] **Implement RPC proxy** (server-side API route that forwards to Helius)
- [ ] **Set up transaction fee monitoring** and treasury reporting
- [ ] **Consider multisig** for treasury authority and admin roles

---

## 11. Recommendations Summary

### Must-Do Before Mainnet (Blockers)
1. **Fix security issues:** Remove hardcoded private key, rotate exposed API key
2. **Create mainnet configuration:** Network config, program deploy, environment variables
3. **Environment-driven config:** Replace all hardcoded devnet values in web/lib/config.ts
4. **Fund deployer and keeper wallets:** ~10 SOL deployer, ~0.5 SOL keeper
5. **Fund treasury:** ≥ $10 USDC in treasury account

### Should-Do Before Mainnet (High Value)
6. **Enable Vercel WAF/DDoS protection** as requested
7. **Server-side RPC proxy** to protect API keys
8. **Set appropriate reserve factor** for revenue generation
9. **Protect the cron endpoint** with CRON_SECRET
10. **Review and tighten risk parameters** for mainnet launch

### Nice-to-Have (Post-Launch)
11. Professional security audit
12. Build proper keeper service
13. Implement 1% borrow origination fee (if reserve factor revenue insufficient)
14. Monitoring and alerting infrastructure
15. Multisig governance for admin operations

---

## Appendix: Cost Summary

| Item | SOL | USD (at $150/SOL) |
|------|-----|-------------------|
| Program deploy | 3–4 | $450–600 |
| Circuit uploads (7) | 2–4 | $300–600 |
| Account creation | 0.015 | ~$2 |
| Transaction fees | 0.15 | ~$23 |
| Operational reserve | 2.0 | $300 |
| Treasury USDC funding | — | $10 |
| **TOTAL** | **~8–10 SOL + $10 USDC** | **~$1,200–1,500 + $10** |

> This is a conservative estimate. Actual costs may be lower if circuit binaries are smaller than estimated, and rent from reclaimed devnet comp defs (~1–2 SOL) can offset mainnet costs.

---

*Report generated from full codebase analysis. All findings are based on the current implementation as the source of truth, not the docs/specs specifications.*
