// Native SOL unwrap (Requirement 5.8; design: Unshield).
//
// SOL unshield delivers wSOL to the owner's wSOL token account. Getting native
// SOL back is a SEPARATE owner-signed transaction, not part of the program:
// SPL Token `CloseAccount` on the wSOL token account sends its lamports (the
// wrapped balance plus the account rent) to the owner and closes it. The
// builders need no RPC. The caller sets the recent blockhash and signs.

import {
  PublicKey,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";

/** The classic SPL Token program. Token-2022 is out of v1 scope. */
export const SPL_TOKEN_PROGRAM_ID = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);

/** SPL Token instruction tag of `CloseAccount`. */
const CLOSE_ACCOUNT_TAG = 9;

/**
 * `CloseAccount` of `wsolAccount`: lamports go to `owner`, who signs as the
 * token account authority.
 */
export function buildUnwrapWsolInstruction(
  owner: PublicKey,
  wsolAccount: PublicKey,
): TransactionInstruction {
  return new TransactionInstruction({
    programId: SPL_TOKEN_PROGRAM_ID,
    keys: [
      { pubkey: wsolAccount, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: true, isWritable: false },
    ],
    data: Buffer.from([CLOSE_ACCOUNT_TAG]),
  });
}

/**
 * A standalone, unsigned transaction that unwraps `wsolAccount` to native SOL
 * for `owner`. The owner is the fee payer and the only signer. Set
 * `recentBlockhash` before signing.
 */
export function buildUnwrapWsolTransaction(
  owner: PublicKey,
  wsolAccount: PublicKey,
): Transaction {
  const transaction = new Transaction();
  transaction.feePayer = owner;
  transaction.add(buildUnwrapWsolInstruction(owner, wsolAccount));
  return transaction;
}
