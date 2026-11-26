# Walnut diagrams

Visual reference for how Walnut works. The system overview and the confidential liquidation flow live in the [README](../README.md#11-architecture). This page adds the flows that were not drawn there. Details and field layouts are in [ARCHITECTURE.md](ARCHITECTURE.md).

Contents: [1. Life of a private operation](#1-life-of-a-private-operation) · [2. Callback decision flow](#2-callback-decision-flow) · [3. Operation Intent states](#3-operation-intent-states) · [4. Viewing Key derivation and rotation](#4-viewing-key-derivation-and-rotation) · [5. Account map](#5-account-map) · [6. Reserve and supply revenue flow](#6-interest-and-reserve-flow)

---

## 1. Life of a private operation

Every confidential action (shield, borrow, repay, supply, withdraw) follows the same path. Example: a private borrow.

```mermaid
sequenceDiagram
    autonumber
    actor User
    participant App as Web app / SDK
    participant Walnut as Walnut program (Solana)
    participant Arcium as Arcium MPC cluster

    User->>App: Borrow 100 USDC
    App->>App: Encrypt amount with x25519 key
    App->>Walnut: queue instruction + encrypted amount
    Walnut->>Walnut: Create Operation Intent, capture generations
    Walnut->>Walnut: Lock Position and Pool (atomic)
    Walnut->>Arcium: Queue computation (pool_op / position_private)
    Arcium->>Arcium: Check collateral, LTV and pool liquidity on secret shares
    Arcium-->>Walnut: Signed callback (new encrypted state + yes/no)
    Walnut->>Walnut: Verify output, classify decision
    alt Applied
        Walnut->>Walnut: Write new ciphertexts, bump generation
        Walnut-->>App: USDC released, Lock cleared
    else Zero / Stale / Expired / Failed
        Walnut->>Walnut: Change nothing, release Lock
        Walnut-->>App: Operation refused, funds safe
    end
    App->>App: Decrypt new balance locally with Viewing Key
```

---

## 2. Callback decision flow

The program checks the Arcium callback in a fixed order before it touches any state.

```mermaid
flowchart TD
    A[Callback arrives] --> B{Output authenticated?}
    B -- No --> LQ[LeaveQueued: Lock stays held]
    B -- Yes --> C{Intent key and kind match?}
    C -- No --> ERR[Revert transaction]
    C -- Yes --> D{Intent still Queued?}
    D -- No --> IGN[Ignore: no change]
    D -- Yes --> E{Offset and commitments match?}
    E -- No --> ERR
    E -- Yes --> F{Static failure reported?}
    F -- Yes --> FAIL[Failed: release Lock]
    F -- No --> G{Generations, index and rate epoch still current?}
    G -- No --> STALE[Stale: release Lock]
    G -- Yes --> H{Slot within Decision Expiry?}
    H -- No --> EXP[Expired: release Lock]
    H -- Yes --> I{Approval amount}
    I -- Zero --> ZERO[Zero: release Lock]
    I -- Non-zero --> APP[Applied: write state, bump generation, release Lock]
```

---

## 3. Operation Intent states

Each queued computation owns one Operation Intent. Only the verified Arcium callback can move it forward, except abandonment, which anyone can trigger after the Lock expires.

```mermaid
stateDiagram-v2
    [*] --> Queued: queue instruction
    Queued --> Applied: callback, state changed
    Queued --> Zero: callback, nothing to apply
    Queued --> Failed: callback reports failure
    Queued --> Stale: state moved on
    Queued --> Expired: decision window passed
    Queued --> Abandoned: anyone, after Lock expiry
    Applied --> [*]
    Zero --> [*]
    Failed --> [*]
    Stale --> [*]
    Expired --> [*]
    Abandoned --> [*]
```

A late callback after `Abandoned` is a no-op: the Locks are cleared, generations have moved on and the Intent is terminal.

---

## 4. Viewing Key derivation and rotation

Your Viewing Key is derived from your wallet signature. It is never sent anywhere.

```mermaid
flowchart LR
    W[Wallet signs fixed message] --> S[64-byte Ed25519 signature]
    S --> H[HKDF-SHA-256]
    H --> K[x25519 Viewing Key]
    K --> D[Decrypt Position in browser]
    K -. public half only .-> P[Position.viewing_pubkey on-chain]
    L[Lock, disconnect, tab hidden] --> Z[Key zeroed in memory]
```

Rotation re-encrypts the position to a new key without revealing any value:

```mermaid
sequenceDiagram
    autonumber
    actor Owner
    participant Walnut as Walnut program
    participant Arcium as Arcium MPC

    Owner->>Owner: Sign message for epoch + 1, derive new key
    Owner->>Walnut: rotate_position_key(new public key, expected old epoch)
    Walnut->>Walnut: Lock Position, create Intent
    Walnut->>Arcium: Queue rotate_position_key circuit
    Arcium->>Arcium: Decrypt under old key, re-seal under new key
    Arcium-->>Walnut: Callback with re-encrypted payload
    Walnut->>Walnut: Write payload, new key and epoch together
    Note over Walnut: Failure leaves the old key valid
```

---

## 5. Account map

```mermaid
flowchart TD
    M[Market PDA] --> AS[Asset State PDA<br/>risk, rates, caps, pauses, timelock]
    M --> PS[Pool State PDA<br/>generation, Lock, encrypted pool payload]
    M --> V1[wSOL vault]
    M --> V2[USDC vault]
    O[Owner wallet] --> POS[Position PDA<br/>5 encrypted values + viewing key]
    O --> OC[Operation Counter PDA]
    OC --> OI[Operation Intent PDA<br/>one per queued computation]
    POS -. locked by .-> OI
    PS -. locked by .-> OI
    AS -. generation captured by .-> OI
```

---

## 6. Interest and reserve flow

How borrower interest splits between suppliers and the protocol (launch reserve factor 10%).

```mermaid
flowchart LR
    B[Borrowers pay interest<br/>via borrow index] --> I[Interest accrued]
    I -->|90%| SUP[Suppliers<br/>via supply index]
    I -->|10% reserve factor| R[Protocol reserves]
    R -->|reserve_withdrawal| T[Treasury USDC account]
```
