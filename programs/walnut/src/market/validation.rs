//! Pure validation for `initialize_market` and the Pool-init computation.
//!
//! Every check is a plain function over plain values so it runs in host tests.
//! The Anchor handler builds the views from its loaded accounts and calls these
//! before it writes anything. Typed Anchor accounts (`Program<Token>`,
//! `Account<Mint>`) reject a wrong token program or a wrong mint owner even
//! earlier; [`check_market_mints`] repeats the owner rule so the rule set is in
//! one tested place.

use anchor_lang::prelude::Pubkey;
use anchor_spl::token::spl_token;

use super::state::MarketStatus;
use super::USDC_DECIMALS;
use crate::state::accounts::PoolStatus;

/// The BPF upgradeable loader (loader v3) program id.
pub const BPF_LOADER_UPGRADEABLE_ID: Pubkey =
    Pubkey::from_str_const("BPFLoaderUpgradeab1e11111111111111111111111");

/// `UpgradeableLoaderState::Program` enum tag (u32 little-endian).
const LOADER_STATE_PROGRAM: u32 = 2;
/// `UpgradeableLoaderState::ProgramData` enum tag (u32 little-endian).
const LOADER_STATE_PROGRAM_DATA: u32 = 3;
/// `Program` layout: tag (4) | programdata_address (32).
const PROGRAM_STATE_LEN: usize = 4 + 32;
/// `ProgramData` header: tag (4) | slot (8) | option tag (1) | authority (32).
const PROGRAM_DATA_HEADER_LEN: usize = 4 + 8 + 1 + 32;

/// Errors of the market initialization checks. Mapped onto appended program
/// `ErrorCode`s in `errors.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketError {
    /// The signer is not the program's upgrade authority, or the program is
    /// not upgradeable.
    InitializerNotUpgradeAuthority,
    /// The program or program-data account is not the expected loader state.
    InvalidProgramDataAccount,
    /// The Market was already initialized.
    MarketAlreadyInitialized,
    /// The Asset State was already initialized.
    AssetStateAlreadyInitialized,
    /// The Pool State was already initialized.
    PoolStateAlreadyInitialized,
    /// The Recovery Position was already initialized.
    RecoveryPositionAlreadyInitialized,
    /// The vault authority was already initialized.
    VaultAuthorityAlreadyInitialized,
    /// A mint is not owned by the classic SPL Token program.
    MintNotOwnedByTokenProgram,
    /// A mint account is not initialized.
    MintNotInitialized,
    /// The wSOL mint is not the native mint.
    WsolMintMismatch,
    /// The USDC mint is the native mint.
    UsdcMintIsNative,
    /// The USDC mint does not use 6 decimals.
    InvalidUsdcDecimals,
    /// An authority key is all zero.
    InvalidAuthority,
    /// The Pool State is not waiting for its encrypted zero state.
    PoolStateNotInitializing,
    // ---- Task 4.2: roles (see `roles.rs`) --------------------------------
    /// The signer is not the market admin.
    NotMarketAdmin,
    /// The signer is not the pending admin.
    NotPendingAdmin,
    /// No admin transfer is pending.
    NoPendingAdmin,
    /// The signer is not the recorded upgrade authority.
    NotUpgradeAuthority,
    /// The signer is not the risk authority.
    NotRiskAuthority,
    /// The signer is not the treasury authority.
    NotTreasuryAuthority,
    /// The signer is not the guardian authority.
    NotGuardianAuthority,
    /// The key is one of the Market's own accounts and cannot hold a role.
    InvalidRoleKey,
    /// The new key equals the current one.
    RoleKeyUnchanged,
    /// The key already holds another separated role.
    RoleKeyNotDistinct,
    /// The loader does not name this key as the program upgrade authority.
    UpgradeAuthorityMismatch,
    /// The Market is not active.
    MarketNotActive,
    // ---- Task 4.3: pauses (see `pause.rs`) -------------------------------
    /// The action's pause category is paused.
    ActionPaused,
    /// The pause mask is empty or names an undefined bit.
    InvalidPauseFlags,
    /// The pause mask names a rescue bit (repay or collateral addition).
    PauseNotAllowed,
    /// The guardian pause duration is zero or above the maximum.
    InvalidPauseDuration,
    /// The guardian must wait for the cooldown after an expired pause.
    GuardianPauseCooldown,
    /// No guardian pause is active (renew) or stored (clear).
    NoActiveGuardianPause,
    /// A renewed guardian expiry must be later than the current one.
    InvalidPauseExpiry,
    // ---- Task 4.4: timelocked configuration (see `config.rs`) -----------
    /// The proposal id is not the Asset State's next proposal id.
    ConfigProposalIdMismatch,
    /// The proposed value equals the current one.
    ConfigUnchanged,
    /// A parameter is outside its absolute bounds.
    ConfigOutOfBounds,
    /// A parameter moves more than its per-execution limit.
    ConfigMovementTooLarge,
    /// The Requirement 4.13 liquidation cross-bound does not hold.
    LiquidationCrossBound,
    /// The timelock delay is outside its bounds.
    TimelockDelayOutOfBounds,
    /// The timelock delay has not elapsed.
    TimelockNotElapsed,
    /// The proposal's execution window has ended.
    ConfigProposalExpired,
    /// The proposal was already executed or cancelled.
    ConfigProposalNotPending,
    /// The changed group no longer holds the value recorded at proposal time.
    ConfigProposalBaseChanged,
    /// The signer is neither the risk authority nor the guardian.
    NotConfigCanceller,
    /// A proposal id, time, or generation would overflow.
    ConfigArithmeticOverflow,
    // ---- Task 4.5: caps and lifecycle counts (see `caps.rs`) -------------
    /// A new Position would exceed `position_count_cap`.
    PositionCountCapReached,
    /// An immediate cap change names a non-cap group or raises a cap; raises
    /// need the timelock.
    CapChangeNotDecrease,
    /// A public lifecycle count or reservation would overflow or underflow.
    LifecycleCountOutOfRange,
    /// An authenticated debt transition contradicts the Position's public
    /// `has_debt` or the operation's direction.
    DebtTransitionMismatch,
}

/// The fields of a mint account that initialization checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MintView {
    /// The mint address.
    pub key: Pubkey,
    /// The program that owns the mint account.
    pub owner: Pubkey,
    /// The mint decimals.
    pub decimals: u8,
    /// Whether the mint is initialized.
    pub is_initialized: bool,
}

/// An account's key, owner, and data, for the loader-state checks.
#[derive(Clone, Copy, Debug)]
pub struct ProgramAccountView<'a> {
    /// The account address.
    pub key: Pubkey,
    /// The account owner program.
    pub owner: Pubkey,
    /// The account data.
    pub data: &'a [u8],
}

fn check_mint_account(mint: &MintView) -> Result<(), MarketError> {
    if mint.owner != spl_token::ID {
        return Err(MarketError::MintNotOwnedByTokenProgram);
    }
    if !mint.is_initialized {
        return Err(MarketError::MintNotInitialized);
    }
    Ok(())
}

/// Validates the market mints: both owned by classic SPL Token and initialized,
/// wSOL is the native mint, and USDC is a different mint with 6 decimals.
///
/// Token-2022 is out of v1 scope, so a Token-2022 mint is rejected.
pub fn check_market_mints(wsol: &MintView, usdc: &MintView) -> Result<(), MarketError> {
    check_mint_account(wsol)?;
    if wsol.key != spl_token::native_mint::ID {
        return Err(MarketError::WsolMintMismatch);
    }
    check_mint_account(usdc)?;
    if usdc.key == spl_token::native_mint::ID {
        return Err(MarketError::UsdcMintIsNative);
    }
    if usdc.decimals != USDC_DECIMALS {
        return Err(MarketError::InvalidUsdcDecimals);
    }
    Ok(())
}

fn read_u32(data: &[u8]) -> u32 {
    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
}

fn read_key(data: &[u8]) -> Pubkey {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&data[..32]);
    Pubkey::new_from_array(bytes)
}

/// Requires `signer` to be the current upgrade authority of `program`.
///
/// `program` must be owned by the upgradeable loader and point at
/// `program_data`, whose header must name `signer` as upgrade authority. An
/// immutable program (no authority) or any other loader is rejected, matching
/// Arcium's rule that only the upgrade authority may initialize the MXE. This
/// stops anyone else from front-running initialization with their own admin or
/// USDC mint.
pub fn check_upgrade_authority(
    program: &ProgramAccountView<'_>,
    program_data: &ProgramAccountView<'_>,
    signer: &Pubkey,
) -> Result<(), MarketError> {
    if program.owner != BPF_LOADER_UPGRADEABLE_ID {
        return Err(MarketError::InitializerNotUpgradeAuthority);
    }
    if program.data.len() < PROGRAM_STATE_LEN || read_u32(program.data) != LOADER_STATE_PROGRAM {
        return Err(MarketError::InvalidProgramDataAccount);
    }
    let program_data_address = read_key(&program.data[4..]);
    if program_data.key != program_data_address
        || program_data.owner != BPF_LOADER_UPGRADEABLE_ID
        || program_data.data.len() < PROGRAM_DATA_HEADER_LEN
        || read_u32(program_data.data) != LOADER_STATE_PROGRAM_DATA
    {
        return Err(MarketError::InvalidProgramDataAccount);
    }
    match program_data.data[12] {
        0 => Err(MarketError::InitializerNotUpgradeAuthority),
        1 if read_key(&program_data.data[13..]) == *signer => Ok(()),
        1 => Err(MarketError::InitializerNotUpgradeAuthority),
        _ => Err(MarketError::InvalidProgramDataAccount),
    }
}

/// Requires every configured authority to be a real (non-zero) key.
pub fn check_initialize_authorities(authorities: &[Pubkey]) -> Result<(), MarketError> {
    if authorities.iter().any(|key| *key == Pubkey::default()) {
        return Err(MarketError::InvalidAuthority);
    }
    Ok(())
}

/// The written-or-not markers of every account `initialize_market` creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitMarkers {
    /// Market status.
    pub market_status: MarketStatus,
    /// Asset State layout version (zero until written).
    pub asset_layout_version: u8,
    /// Pool State status.
    pub pool_status: PoolStatus,
    /// Recovery Position owner (zero until written).
    pub recovery_position_owner: Pubkey,
    /// Vault authority market (zero until written).
    pub vault_authority_market: Pubkey,
}

/// Rejects repeat initialization (Requirement 4.2): every account must still be
/// in its zeroed, never-written state. Each account has its own error code.
pub fn ensure_fresh(markers: &InitMarkers) -> Result<(), MarketError> {
    if markers.market_status != MarketStatus::Uninitialized {
        return Err(MarketError::MarketAlreadyInitialized);
    }
    if markers.asset_layout_version != 0 {
        return Err(MarketError::AssetStateAlreadyInitialized);
    }
    if markers.pool_status != PoolStatus::Uninitialized {
        return Err(MarketError::PoolStateAlreadyInitialized);
    }
    if markers.recovery_position_owner != Pubkey::default() {
        return Err(MarketError::RecoveryPositionAlreadyInitialized);
    }
    if markers.vault_authority_market != Pubkey::default() {
        return Err(MarketError::VaultAuthorityAlreadyInitialized);
    }
    Ok(())
}

/// Requires the Pool State to be waiting for its encrypted zero state, so the
/// Pool-init computation is queued only before activation.
pub fn check_pool_initializing(status: PoolStatus) -> Result<(), MarketError> {
    if status != PoolStatus::Initializing {
        return Err(MarketError::PoolStateNotInitializing);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN_2022: Pubkey =
        Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    fn wsol() -> MintView {
        MintView {
            key: spl_token::native_mint::ID,
            owner: spl_token::ID,
            decimals: 9,
            is_initialized: true,
        }
    }

    fn usdc() -> MintView {
        MintView {
            key: key(0x55),
            owner: spl_token::ID,
            decimals: 6,
            is_initialized: true,
        }
    }

    fn program_state(program_data: &Pubkey) -> Vec<u8> {
        let mut data = LOADER_STATE_PROGRAM.to_le_bytes().to_vec();
        data.extend(program_data.to_bytes());
        data
    }

    fn program_data_state(authority: Option<Pubkey>) -> Vec<u8> {
        let mut data = LOADER_STATE_PROGRAM_DATA.to_le_bytes().to_vec();
        data.extend(42u64.to_le_bytes());
        match authority {
            Some(key) => {
                data.push(1);
                data.extend(key.to_bytes());
            }
            None => {
                data.push(0);
                data.extend([0u8; 32]);
            }
        }
        // Program bytes follow the header.
        data.extend([0xCC; 16]);
        data
    }

    fn fresh() -> InitMarkers {
        InitMarkers {
            market_status: MarketStatus::Uninitialized,
            asset_layout_version: 0,
            pool_status: PoolStatus::Uninitialized,
            recovery_position_owner: Pubkey::default(),
            vault_authority_market: Pubkey::default(),
        }
    }

    // ---- Mints -------------------------------------------------------------

    #[test]
    fn the_canonical_mint_pair_is_accepted() {
        assert_eq!(check_market_mints(&wsol(), &usdc()), Ok(()));
    }

    #[test]
    fn mints_owned_by_another_program_are_rejected() {
        for owner in [TOKEN_2022, Pubkey::default(), crate::ID] {
            let wrong_wsol = MintView { owner, ..wsol() };
            assert_eq!(
                check_market_mints(&wrong_wsol, &usdc()),
                Err(MarketError::MintNotOwnedByTokenProgram)
            );
            let wrong_usdc = MintView { owner, ..usdc() };
            assert_eq!(
                check_market_mints(&wsol(), &wrong_usdc),
                Err(MarketError::MintNotOwnedByTokenProgram)
            );
        }
    }

    #[test]
    fn uninitialized_mints_are_rejected() {
        let wsol_uninit = MintView {
            is_initialized: false,
            ..wsol()
        };
        assert_eq!(
            check_market_mints(&wsol_uninit, &usdc()),
            Err(MarketError::MintNotInitialized)
        );
        let usdc_uninit = MintView {
            is_initialized: false,
            ..usdc()
        };
        assert_eq!(
            check_market_mints(&wsol(), &usdc_uninit),
            Err(MarketError::MintNotInitialized)
        );
    }

    #[test]
    fn wsol_must_be_the_native_mint() {
        let fake = MintView {
            key: key(0x99),
            ..wsol()
        };
        assert_eq!(
            check_market_mints(&fake, &usdc()),
            Err(MarketError::WsolMintMismatch)
        );
    }

    #[test]
    fn usdc_must_not_be_native_and_must_have_six_decimals() {
        assert_eq!(
            check_market_mints(&wsol(), &wsol()),
            Err(MarketError::UsdcMintIsNative)
        );
        for decimals in [0, 5, 7, 9, 255] {
            let wrong = MintView { decimals, ..usdc() };
            assert_eq!(
                check_market_mints(&wsol(), &wrong),
                Err(MarketError::InvalidUsdcDecimals)
            );
        }
    }

    // ---- Upgrade authority -------------------------------------------------

    #[test]
    fn the_upgrade_authority_may_initialize() {
        let program_data_key = key(0x70);
        let program = program_state(&program_data_key);
        let program_data = program_data_state(Some(key(1)));
        let result = check_upgrade_authority(
            &ProgramAccountView {
                key: crate::ID,
                owner: BPF_LOADER_UPGRADEABLE_ID,
                data: &program,
            },
            &ProgramAccountView {
                key: program_data_key,
                owner: BPF_LOADER_UPGRADEABLE_ID,
                data: &program_data,
            },
            &key(1),
        );
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn anyone_else_or_an_immutable_program_is_rejected() {
        let program_data_key = key(0x70);
        let program = program_state(&program_data_key);
        let view = |data: &[u8]| -> Result<(), MarketError> {
            check_upgrade_authority(
                &ProgramAccountView {
                    key: crate::ID,
                    owner: BPF_LOADER_UPGRADEABLE_ID,
                    data: &program,
                },
                &ProgramAccountView {
                    key: program_data_key,
                    owner: BPF_LOADER_UPGRADEABLE_ID,
                    data,
                },
                &key(1),
            )
        };
        assert_eq!(
            view(&program_data_state(Some(key(2)))),
            Err(MarketError::InitializerNotUpgradeAuthority)
        );
        assert_eq!(
            view(&program_data_state(None)),
            Err(MarketError::InitializerNotUpgradeAuthority)
        );
        let mut bad_option = program_data_state(Some(key(1)));
        bad_option[12] = 2;
        assert_eq!(
            view(&bad_option),
            Err(MarketError::InvalidProgramDataAccount)
        );
        let mut bad_tag = program_data_state(Some(key(1)));
        bad_tag[0] = 2;
        assert_eq!(view(&bad_tag), Err(MarketError::InvalidProgramDataAccount));
        assert_eq!(
            view(&program_data_state(Some(key(1)))[..20]),
            Err(MarketError::InvalidProgramDataAccount)
        );
    }

    #[test]
    fn a_substituted_program_data_account_is_rejected() {
        let program = program_state(&key(0x70));
        let program_data = program_data_state(Some(key(1)));
        for (data_key, data_owner) in [
            (key(0x71), BPF_LOADER_UPGRADEABLE_ID),
            (key(0x70), crate::ID),
        ] {
            let result = check_upgrade_authority(
                &ProgramAccountView {
                    key: crate::ID,
                    owner: BPF_LOADER_UPGRADEABLE_ID,
                    data: &program,
                },
                &ProgramAccountView {
                    key: data_key,
                    owner: data_owner,
                    data: &program_data,
                },
                &key(1),
            );
            assert_eq!(result, Err(MarketError::InvalidProgramDataAccount));
        }
    }

    #[test]
    fn a_program_under_another_loader_is_rejected() {
        let program = program_state(&key(0x70));
        let program_data = program_data_state(Some(key(1)));
        let bpf_loader_v2 = Pubkey::from_str_const("BPFLoader2111111111111111111111111111111111");
        let result = check_upgrade_authority(
            &ProgramAccountView {
                key: crate::ID,
                owner: bpf_loader_v2,
                data: &program,
            },
            &ProgramAccountView {
                key: key(0x70),
                owner: BPF_LOADER_UPGRADEABLE_ID,
                data: &program_data,
            },
            &key(1),
        );
        assert_eq!(result, Err(MarketError::InitializerNotUpgradeAuthority));

        let mut not_program = program.clone();
        not_program[0] = 3;
        let result = check_upgrade_authority(
            &ProgramAccountView {
                key: crate::ID,
                owner: BPF_LOADER_UPGRADEABLE_ID,
                data: &not_program,
            },
            &ProgramAccountView {
                key: key(0x70),
                owner: BPF_LOADER_UPGRADEABLE_ID,
                data: &program_data,
            },
            &key(1),
        );
        assert_eq!(result, Err(MarketError::InvalidProgramDataAccount));
    }

    // ---- Authorities and repeat initialization -----------------------------

    #[test]
    fn zero_authorities_are_rejected() {
        assert_eq!(check_initialize_authorities(&[key(1), key(2)]), Ok(()));
        assert_eq!(
            check_initialize_authorities(&[key(1), Pubkey::default()]),
            Err(MarketError::InvalidAuthority)
        );
    }

    #[test]
    fn every_written_account_has_its_own_repeat_error() {
        assert_eq!(ensure_fresh(&fresh()), Ok(()));
        let cases = [
            (
                InitMarkers {
                    market_status: MarketStatus::Active,
                    ..fresh()
                },
                MarketError::MarketAlreadyInitialized,
            ),
            (
                InitMarkers {
                    asset_layout_version: 1,
                    ..fresh()
                },
                MarketError::AssetStateAlreadyInitialized,
            ),
            (
                InitMarkers {
                    pool_status: PoolStatus::Initializing,
                    ..fresh()
                },
                MarketError::PoolStateAlreadyInitialized,
            ),
            (
                InitMarkers {
                    pool_status: PoolStatus::Active,
                    ..fresh()
                },
                MarketError::PoolStateAlreadyInitialized,
            ),
            (
                InitMarkers {
                    recovery_position_owner: key(3),
                    ..fresh()
                },
                MarketError::RecoveryPositionAlreadyInitialized,
            ),
            (
                InitMarkers {
                    vault_authority_market: key(4),
                    ..fresh()
                },
                MarketError::VaultAuthorityAlreadyInitialized,
            ),
        ];
        for (markers, expected) in cases {
            assert_eq!(ensure_fresh(&markers), Err(expected));
        }
    }

    #[test]
    fn pool_init_is_only_queued_while_initializing() {
        assert_eq!(check_pool_initializing(PoolStatus::Initializing), Ok(()));
        for status in [PoolStatus::Uninitialized, PoolStatus::Active] {
            assert_eq!(
                check_pool_initializing(status),
                Err(MarketError::PoolStateNotInitializing)
            );
        }
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

        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[(self.sample() % items.len() as u64) as usize]
        }
    }

    /// **Validates: Requirements 4.1** - over random mint keys, owners,
    /// decimals, and initialization flags, the mint pair is accepted exactly
    /// when both are initialized classic SPL Token mints, wSOL is the native
    /// mint, and USDC is a different 6-decimal mint.
    #[test]
    fn prop_mint_validation_accepts_only_the_canonical_pair() {
        let mut rng = Lcg(0x5EED_4101);
        let keys = [spl_token::native_mint::ID, key(0x55), key(0x56)];
        let owners = [spl_token::ID, TOKEN_2022, Pubkey::default()];
        let decimals = [0u8, 6, 9];
        let mut accepted = 0;
        for _ in 0..5_000 {
            let wsol = MintView {
                key: rng.pick(&keys),
                owner: rng.pick(&owners),
                decimals: rng.pick(&decimals),
                is_initialized: !rng.sample().is_multiple_of(4),
            };
            let usdc = MintView {
                key: rng.pick(&keys),
                owner: rng.pick(&owners),
                decimals: rng.pick(&decimals),
                is_initialized: !rng.sample().is_multiple_of(4),
            };
            let valid = wsol.owner == spl_token::ID
                && wsol.is_initialized
                && wsol.key == spl_token::native_mint::ID
                && usdc.owner == spl_token::ID
                && usdc.is_initialized
                && usdc.key != spl_token::native_mint::ID
                && usdc.decimals == 6;
            let result = check_market_mints(&wsol, &usdc);
            assert_eq!(result.is_ok(), valid, "{wsol:?} {usdc:?} {result:?}");
            accepted += usize::from(valid);
        }
        assert!(accepted > 0, "the generator must reach the accepted case");
    }

    /// **Validates: Requirements 4.1** - over random signers, recorded
    /// authorities, loader owners, and program-data addresses, initialization
    /// is allowed exactly when the signer is the recorded upgrade authority of
    /// an upgradeable program whose program-data account matches.
    #[test]
    fn prop_only_the_upgrade_authority_may_initialize() {
        let mut rng = Lcg(0x5EED_4102);
        let people = [key(1), key(2), key(3)];
        let owners = [BPF_LOADER_UPGRADEABLE_ID, crate::ID];
        let data_keys = [key(0x70), key(0x71)];
        let mut accepted = 0;
        for _ in 0..5_000 {
            let signer = rng.pick(&people);
            let authority = if rng.sample().is_multiple_of(5) {
                None
            } else {
                Some(rng.pick(&people))
            };
            let program_owner = rng.pick(&owners);
            let data_owner = rng.pick(&owners);
            let data_key = rng.pick(&data_keys);
            let program = program_state(&key(0x70));
            let program_data = program_data_state(authority);
            let result = check_upgrade_authority(
                &ProgramAccountView {
                    key: crate::ID,
                    owner: program_owner,
                    data: &program,
                },
                &ProgramAccountView {
                    key: data_key,
                    owner: data_owner,
                    data: &program_data,
                },
                &signer,
            );
            let valid = program_owner == BPF_LOADER_UPGRADEABLE_ID
                && data_owner == BPF_LOADER_UPGRADEABLE_ID
                && data_key == key(0x70)
                && authority == Some(signer);
            assert_eq!(result.is_ok(), valid);
            accepted += usize::from(valid);
        }
        assert!(accepted > 0, "the generator must reach the accepted case");
    }

    /// **Validates: Requirements 4.2** - any combination of already-written
    /// accounts is rejected, and only the all-fresh state is accepted.
    #[test]
    fn prop_any_written_account_blocks_initialization() {
        for mask in 0u8..32 {
            let markers = InitMarkers {
                market_status: if mask & 1 != 0 {
                    MarketStatus::Active
                } else {
                    MarketStatus::Uninitialized
                },
                asset_layout_version: (mask >> 1) & 1,
                pool_status: if mask & 4 != 0 {
                    PoolStatus::Active
                } else {
                    PoolStatus::Uninitialized
                },
                recovery_position_owner: if mask & 8 != 0 {
                    key(8)
                } else {
                    Pubkey::default()
                },
                vault_authority_market: if mask & 16 != 0 {
                    key(9)
                } else {
                    Pubkey::default()
                },
            };
            assert_eq!(ensure_fresh(&markers).is_ok(), mask == 0, "mask {mask}");
        }
    }
}
