//! Canonical Operation Intent commitments (Requirement 3.3).
//!
//! An Operation Intent binds two 32-byte SHA-256 commitments:
//!
//! * `accounts_commitment` covers every ordered account the operation touches,
//!   with its role, expected owner, mint, authority, writability, and captured
//!   generation ([`account_commitment`]).
//! * `inputs_commitment` covers the operation kind and every immutable public or
//!   encrypted input ([`input_commitment`]).
//!
//! The queue instruction computes both and stores them on the Intent. The
//! callback recomputes both from its own context and compares, so an account or
//! an input swapped between queue and callback changes a commitment.
//!
//! # Canonical serialization
//!
//! Every integer is little-endian. Every value is either fixed width or carries
//! an explicit length or presence flag, so the byte stream decodes in exactly
//! one way. Distinct inputs therefore never serialize to the same stream, and
//! SHA-256 collision resistance does the rest. Each stream starts with a
//! domain-separation header so the two commitment kinds (and any future
//! versions) can never collide with each other:
//!
//! ```text
//! header   = domain_len (u8) | domain (ASCII) | version (u8 = 1)
//!
//! accounts = header(ACCOUNTS_COMMITMENT_DOMAIN)
//!            | count (u32) | account * count
//! account  = key (32) | role (u8) | expected_owner (32)
//!            | mint (opt) | authority (opt) | writable (u8: 0 or 1)
//!            | generation (opt u64)
//! opt<T>   = 0x00                  (absent)
//!          | 0x01 | T              (present; Pubkey 32 bytes, u64 8 bytes)
//!
//! inputs   = header(INPUTS_COMMITMENT_DOMAIN)
//!            | kind (u8) | count (u32) | field * count
//! field    = id (u8) | type (u8) | value
//! value    = Bool: u8 | U8: u8 | U64: 8 | I64: 8 | U128: 16 | Key: 32
//!            | Bytes32: 32 | Bytes: len (u32) | data
//! ```
//!
//! Account order and input field order are part of the commitment, so each
//! operation kind defines one fixed order. Field ids are chosen per operation
//! kind by the instruction that builds the Intent and are append-only.
//!
//! The hash is plain SHA-256 (`super::sha256`), so a client can reproduce it
//! with any SHA-256 implementation.

use anchor_lang::prelude::Pubkey;

use super::intent::OperationKind;
use super::sha256::Sha256;

/// Serialization version, written into every header.
pub const COMMITMENT_VERSION: u8 = 1;

/// Domain-separation tag for `accounts_commitment`.
pub const ACCOUNTS_COMMITMENT_DOMAIN: &[u8] = b"walnut-solana/accounts-commitment";

/// Domain-separation tag for `inputs_commitment`.
pub const INPUTS_COMMITMENT_DOMAIN: &[u8] = b"walnut-solana/inputs-commitment";

/// The role an account plays in an operation.
///
/// Tags are explicit and append-only: they are hashed into the commitment, so
/// renumbering a role would change every commitment that uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AccountRole {
    /// The operation owner's signer.
    Owner = 1,
    /// The rent and fee payer.
    Payer = 2,
    /// The primary Position.
    Position = 3,
    /// The second Position (for example the liquidator).
    SecondPosition = 4,
    /// The Pool State.
    Pool = 5,
    /// The Market.
    Market = 6,
    /// The Asset State.
    AssetState = 7,
    /// The Operation Intent itself.
    OperationIntent = 8,
    /// A custody vault.
    Vault = 9,
    /// A token mint.
    Mint = 10,
    /// A user or treasury token account.
    TokenAccount = 11,
    /// The oracle price update.
    Oracle = 12,
    /// A Liquidation Record.
    LiquidationRecord = 13,
    /// A Recovery Claim.
    RecoveryClaim = 14,
    /// A Pending Shield.
    PendingShield = 15,
    /// A Pending Recovery Settlement.
    PendingRecoverySettlement = 16,
    /// A Disclosure.
    Disclosure = 17,
    /// Any account whose role a later operation defines.
    Other = 255,
}

impl AccountRole {
    /// The byte hashed into the commitment.
    pub const fn tag(self) -> u8 {
        self as u8
    }
}

/// One account as bound by `accounts_commitment`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccountBinding {
    /// The account's address.
    pub key: Pubkey,
    /// The role it plays.
    pub role: AccountRole,
    /// The program that must own it.
    pub expected_owner: Pubkey,
    /// The token mint, for token accounts and mints.
    pub mint: Option<Pubkey>,
    /// The authority that controls it, where one applies.
    pub authority: Option<Pubkey>,
    /// Whether the operation may write it.
    pub writable: bool,
    /// The captured generation, for protected accounts.
    pub generation: Option<u64>,
}

/// The value of one input field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputValue<'a> {
    /// A boolean flag.
    Bool(bool),
    /// An 8-bit integer (for example an enum tag or a mask).
    U8(u8),
    /// An unsigned 64-bit integer (amounts, epochs, slots).
    U64(u64),
    /// A signed 64-bit integer (timestamps, publish times).
    I64(i64),
    /// An unsigned 128-bit integer (indexes, nonces).
    U128(u128),
    /// A public key (destinations, operation-specific PDAs).
    Key(Pubkey),
    /// A fixed 32-byte value (a ciphertext, a public key, or a feed id).
    Bytes32([u8; 32]),
    /// Variable-length bytes (ciphertext metadata).
    Bytes(&'a [u8]),
}

impl InputValue<'_> {
    /// The type byte hashed ahead of the value.
    const fn type_tag(&self) -> u8 {
        match self {
            InputValue::Bool(_) => 1,
            InputValue::U8(_) => 2,
            InputValue::U64(_) => 3,
            InputValue::I64(_) => 4,
            InputValue::U128(_) => 5,
            InputValue::Key(_) => 6,
            InputValue::Bytes32(_) => 7,
            InputValue::Bytes(_) => 8,
        }
    }
}

/// One immutable input of an operation, identified by a per-operation id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputField<'a> {
    /// The field id, unique within the operation kind.
    pub id: u8,
    /// The field value.
    pub value: InputValue<'a>,
}

impl<'a> InputField<'a> {
    /// Creates a field.
    pub const fn new(id: u8, value: InputValue<'a>) -> Self {
        Self { id, value }
    }
}

/// Writes the domain-separation header.
fn put_header(hasher: &mut Sha256, domain: &[u8]) {
    hasher.update(&[domain.len() as u8]);
    hasher.update(domain);
    hasher.update(&[COMMITMENT_VERSION]);
}

fn put_key(hasher: &mut Sha256, key: &Pubkey) {
    hasher.update(&key.to_bytes());
}

fn put_optional_key(hasher: &mut Sha256, key: Option<Pubkey>) {
    match key {
        Some(key) => {
            hasher.update(&[1]);
            put_key(hasher, &key);
        }
        None => hasher.update(&[0]),
    }
}

fn put_optional_u64(hasher: &mut Sha256, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hasher.update(&value.to_le_bytes());
        }
        None => hasher.update(&[0]),
    }
}

/// Computes `accounts_commitment` over the ordered account list.
///
/// Order matters: the same accounts in a different order produce a different
/// commitment. See the module docs for the exact serialization.
pub fn account_commitment(accounts: &[AccountBinding]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    put_header(&mut hasher, ACCOUNTS_COMMITMENT_DOMAIN);
    hasher.update(&(accounts.len() as u32).to_le_bytes());
    for account in accounts {
        put_key(&mut hasher, &account.key);
        hasher.update(&[account.role.tag()]);
        put_key(&mut hasher, &account.expected_owner);
        put_optional_key(&mut hasher, account.mint);
        put_optional_key(&mut hasher, account.authority);
        hasher.update(&[u8::from(account.writable)]);
        put_optional_u64(&mut hasher, account.generation);
    }
    hasher.finalize()
}

/// Computes `inputs_commitment` over the operation kind and ordered input
/// fields. See the module docs for the exact serialization.
pub fn input_commitment(kind: OperationKind, fields: &[InputField<'_>]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    put_header(&mut hasher, INPUTS_COMMITMENT_DOMAIN);
    hasher.update(&[kind.tag()]);
    hasher.update(&(fields.len() as u32).to_le_bytes());
    for field in fields {
        hasher.update(&[field.id, field.value.type_tag()]);
        match &field.value {
            InputValue::Bool(value) => hasher.update(&[u8::from(*value)]),
            InputValue::U8(value) => hasher.update(&[*value]),
            InputValue::U64(value) => hasher.update(&value.to_le_bytes()),
            InputValue::I64(value) => hasher.update(&value.to_le_bytes()),
            InputValue::U128(value) => hasher.update(&value.to_le_bytes()),
            InputValue::Key(value) => put_key(&mut hasher, value),
            InputValue::Bytes32(value) => hasher.update(value),
            InputValue::Bytes(value) => {
                hasher.update(&(value.len() as u32).to_le_bytes());
                hasher.update(value);
            }
        }
    }
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn binding(n: u8) -> AccountBinding {
        AccountBinding {
            key: key(n),
            role: AccountRole::Position,
            expected_owner: key(100),
            mint: None,
            authority: None,
            writable: true,
            generation: Some(u64::from(n)),
        }
    }

    fn sample_accounts() -> Vec<AccountBinding> {
        vec![
            AccountBinding {
                key: key(1),
                role: AccountRole::Position,
                expected_owner: key(0xA0),
                mint: None,
                authority: Some(key(0xB0)),
                writable: true,
                generation: Some(7),
            },
            AccountBinding {
                key: key(2),
                role: AccountRole::Vault,
                expected_owner: key(0xA1),
                mint: Some(key(0xC0)),
                authority: Some(key(0xB1)),
                writable: false,
                generation: None,
            },
        ]
    }

    fn sample_fields() -> Vec<InputField<'static>> {
        vec![
            InputField::new(1, InputValue::U64(1_000_000)),
            InputField::new(2, InputValue::Key(key(9))),
            InputField::new(3, InputValue::Bytes(&[1, 2, 3])),
            InputField::new(4, InputValue::U128(1 << 70)),
            InputField::new(5, InputValue::I64(-42)),
            InputField::new(6, InputValue::Bool(true)),
            InputField::new(7, InputValue::U8(9)),
            InputField::new(8, InputValue::Bytes32([0x5A; 32])),
        ]
    }

    // ---- Golden vectors ----------------------------------------------------
    //
    // The expected digests were produced by an independent Python encoder that
    // follows the module-level serialization table and `hashlib.sha256`, so
    // they pin both the byte format and the hash. Any change to the encoding
    // must change a version, not these vectors.

    #[test]
    fn empty_account_commitment_matches_the_golden_vector() {
        assert_eq!(
            hex(&account_commitment(&[])),
            "a61889b8fc714c6dade4299be8536628edf6a7c7ddfdcc21547fd2dd379d00b2"
        );
    }

    #[test]
    fn account_commitment_matches_the_golden_vector() {
        assert_eq!(
            hex(&account_commitment(&sample_accounts())),
            "2c9fc29c62d59cbf247308555de9c1f792288b1fd64d8f995ea1c2114eb17b50"
        );
    }

    #[test]
    fn input_commitment_matches_the_golden_vector() {
        assert_eq!(
            hex(&input_commitment(OperationKind::Borrow, &sample_fields())),
            "b77809bc22cc78f2c5cfc89d38aa153a57d2a56b27587bed11c84a467990f7bf"
        );
    }

    #[test]
    fn empty_input_commitment_matches_the_golden_vector() {
        assert_eq!(
            hex(&input_commitment(OperationKind::Probe, &[])),
            "2d2381e7e714eca8c231fdc52fc31ca1ced552b65a37df49b898f345e7143873"
        );
    }

    // ---- Properties of the construction -----------------------------------

    #[test]
    fn commitments_are_deterministic() {
        assert_eq!(
            account_commitment(&sample_accounts()),
            account_commitment(&sample_accounts())
        );
        assert_eq!(
            input_commitment(OperationKind::Borrow, &sample_fields()),
            input_commitment(OperationKind::Borrow, &sample_fields())
        );
    }

    #[test]
    fn account_order_changes_the_commitment() {
        let forward = vec![binding(1), binding(2), binding(3)];
        let mut reversed = forward.clone();
        reversed.reverse();
        assert_ne!(account_commitment(&forward), account_commitment(&reversed));

        let mut swapped = forward.clone();
        swapped.swap(0, 1);
        assert_ne!(account_commitment(&forward), account_commitment(&swapped));
    }

    #[test]
    fn input_order_changes_the_commitment() {
        let forward = sample_fields();
        let mut swapped = forward.clone();
        swapped.swap(0, 1);
        assert_ne!(
            input_commitment(OperationKind::Borrow, &forward),
            input_commitment(OperationKind::Borrow, &swapped)
        );
    }

    #[test]
    fn dropping_or_adding_an_entry_changes_the_commitment() {
        let accounts = sample_accounts();
        assert_ne!(
            account_commitment(&accounts),
            account_commitment(&accounts[..1])
        );
        let mut extended = accounts.clone();
        extended.push(binding(3));
        assert_ne!(account_commitment(&accounts), account_commitment(&extended));

        let fields = sample_fields();
        assert_ne!(
            input_commitment(OperationKind::Borrow, &fields),
            input_commitment(OperationKind::Borrow, &fields[..7])
        );
    }

    #[test]
    fn the_two_commitment_kinds_are_domain_separated() {
        // Both empty: only the domain header differs.
        assert_ne!(
            account_commitment(&[]),
            input_commitment(OperationKind::Probe, &[])
        );
        assert_ne!(ACCOUNTS_COMMITMENT_DOMAIN, INPUTS_COMMITMENT_DOMAIN);
    }

    #[test]
    fn the_domain_header_is_hashed() {
        // Recompute the empty accounts commitment by hand with a different
        // domain and confirm it differs from the real one.
        let mut other = Sha256::new();
        put_header(&mut other, b"walnut-solana/other");
        other.update(&0u32.to_le_bytes());
        assert_ne!(other.finalize(), account_commitment(&[]));
    }

    #[test]
    fn operation_kind_changes_the_input_commitment() {
        let fields = sample_fields();
        let borrow = input_commitment(OperationKind::Borrow, &fields);
        let repay = input_commitment(OperationKind::Repay, &fields);
        assert_ne!(borrow, repay);
    }

    #[test]
    fn every_single_account_field_is_bound() {
        let base = sample_accounts();
        let reference = account_commitment(&base);

        let mutations: [fn(&mut AccountBinding); 7] = [
            |a| a.key = key(0xEE),
            |a| a.role = AccountRole::Pool,
            |a| a.expected_owner = key(0xEE),
            |a| a.mint = Some(key(0xEE)),
            |a| a.authority = Some(key(0xEE)),
            |a| a.writable = !a.writable,
            |a| a.generation = Some(a.generation.map_or(1, |g| g + 1)),
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            for position in 0..base.len() {
                let mut changed = base.clone();
                mutate(&mut changed[position]);
                assert_ne!(
                    account_commitment(&changed),
                    reference,
                    "mutation {index} of account {position} was not detected"
                );
            }
        }

        // Absent versus present optional values are distinguished.
        let mut with_generation = base.clone();
        with_generation[1].generation = Some(0);
        assert_ne!(account_commitment(&with_generation), reference);
    }

    #[test]
    fn every_single_input_value_is_bound() {
        let base = sample_fields();
        let reference = input_commitment(OperationKind::Borrow, &base);

        let replacements = [
            InputValue::U64(1_000_001),
            InputValue::Key(key(10)),
            InputValue::Bytes(&[1, 2, 4]),
            InputValue::U128((1 << 70) + 1),
            InputValue::I64(-41),
            InputValue::Bool(false),
            InputValue::U8(10),
            InputValue::Bytes32([0x5B; 32]),
        ];
        for (index, replacement) in replacements.into_iter().enumerate() {
            let mut changed = base.clone();
            changed[index].value = replacement;
            assert_ne!(
                input_commitment(OperationKind::Borrow, &changed),
                reference,
                "value of field {index} was not detected"
            );
        }

        // The field id and the value type are bound as well.
        let mut renumbered = base.clone();
        renumbered[0].id = 99;
        assert_ne!(
            input_commitment(OperationKind::Borrow, &renumbered),
            reference
        );
        let mut retyped = base.clone();
        retyped[0].value = InputValue::I64(1_000_000);
        assert_ne!(input_commitment(OperationKind::Borrow, &retyped), reference);
    }

    #[test]
    fn variable_length_bytes_cannot_shift_into_the_next_field() {
        // Without a length prefix these two would serialize identically.
        let first = [
            InputField::new(1, InputValue::Bytes(&[1, 2])),
            InputField::new(2, InputValue::Bytes(&[3])),
        ];
        let second = [
            InputField::new(1, InputValue::Bytes(&[1])),
            InputField::new(2, InputValue::Bytes(&[2, 3])),
        ];
        assert_ne!(
            input_commitment(OperationKind::Supply, &first),
            input_commitment(OperationKind::Supply, &second)
        );
    }

    #[test]
    fn role_tags_are_stable() {
        assert_eq!(AccountRole::Owner.tag(), 1);
        assert_eq!(AccountRole::Position.tag(), 3);
        assert_eq!(AccountRole::Pool.tag(), 5);
        assert_eq!(AccountRole::Vault.tag(), 9);
        assert_eq!(AccountRole::Other.tag(), 255);
    }

    // ---- Property tests (run via `cargo test -- prop_`) -------------------

    /// Deterministic linear-congruential sampler (no external crate).
    struct Lcg(u64);
    impl Lcg {
        fn sample(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }

        fn key(&mut self) -> Pubkey {
            let mut bytes = [0u8; 32];
            bytes[0..8].copy_from_slice(&self.sample().to_le_bytes());
            bytes[8..16].copy_from_slice(&self.sample().to_le_bytes());
            bytes[16..24].copy_from_slice(&self.sample().to_le_bytes());
            bytes[24..32].copy_from_slice(&self.sample().to_le_bytes());
            Pubkey::new_from_array(bytes)
        }

        fn optional_key(&mut self) -> Option<Pubkey> {
            if self.sample().is_multiple_of(2) {
                None
            } else {
                Some(self.key())
            }
        }

        fn account(&mut self) -> AccountBinding {
            let roles = [
                AccountRole::Owner,
                AccountRole::Position,
                AccountRole::Pool,
                AccountRole::Vault,
                AccountRole::Other,
            ];
            AccountBinding {
                key: self.key(),
                role: roles[(self.sample() % 5) as usize],
                expected_owner: self.key(),
                mint: self.optional_key(),
                authority: self.optional_key(),
                writable: self.sample().is_multiple_of(2),
                generation: if self.sample().is_multiple_of(2) {
                    None
                } else {
                    Some(self.sample())
                },
            }
        }

        fn accounts(&mut self) -> Vec<AccountBinding> {
            let count = (self.sample() % 6) as usize;
            (0..count).map(|_| self.account()).collect()
        }
    }

    /// **Validates: Requirements 3.3** Ã¢â‚¬â€ substituting any one account, in any
    /// position, with a different account changes `accounts_commitment`, and
    /// reordering two distinct accounts changes it too.
    #[test]
    fn prop_account_commitment_resists_substitution_and_reordering() {
        let mut rng = Lcg(0xC0FF_EE01);
        for _ in 0..400 {
            let mut accounts = rng.accounts();
            if accounts.is_empty() {
                accounts.push(rng.account());
            }
            let reference = account_commitment(&accounts);
            assert_eq!(reference, account_commitment(&accounts.clone()));

            let position = (rng.sample() as usize) % accounts.len();
            let mut substituted = accounts.clone();
            substituted[position].key = rng.key();
            if substituted != accounts {
                assert_ne!(account_commitment(&substituted), reference);
            }

            if accounts.len() >= 2 {
                let other = (position + 1) % accounts.len();
                let mut swapped = accounts.clone();
                swapped.swap(position, other);
                if swapped != accounts {
                    assert_ne!(account_commitment(&swapped), reference);
                }
            }
        }
    }

    /// **Validates: Requirements 3.3** Ã¢â‚¬â€ flipping any single byte of any input
    /// changes `inputs_commitment`, and no two distinct random inputs collide.
    #[test]
    fn prop_input_commitment_resists_byte_substitution() {
        let mut rng = Lcg(0xFEED_0002);
        for _ in 0..300 {
            let length = (rng.sample() % 90) as usize;
            let data: Vec<u8> = (0..length).map(|_| rng.sample() as u8).collect();
            let amount = rng.sample();
            let build = |data: &[u8], amount: u64| {
                input_commitment(
                    OperationKind::Supply,
                    &[
                        InputField::new(1, InputValue::U64(amount)),
                        InputField::new(2, InputValue::Bytes(data)),
                    ],
                )
            };
            let reference = build(&data, amount);
            assert_eq!(reference, build(&data, amount));

            for index in 0..data.len() {
                let mut flipped = data.clone();
                flipped[index] ^= 1 << (rng.sample() % 8);
                assert_ne!(build(&flipped, amount), reference);
            }
            for bit in 0..64 {
                assert_ne!(build(&data, amount ^ (1u64 << bit)), reference);
            }
        }
    }

    /// **Validates: Requirements 3.3** Ã¢â‚¬â€ the two commitment kinds never
    /// collide, even over the same bytes.
    #[test]
    fn prop_commitment_kinds_never_collide() {
        let mut rng = Lcg(0xD0D0_0003);
        for _ in 0..200 {
            let accounts = rng.accounts();
            let fields = [InputField::new(1, InputValue::U64(rng.sample()))];
            assert_ne!(
                account_commitment(&accounts),
                input_commitment(OperationKind::Borrow, &fields)
            );
        }
    }
}
