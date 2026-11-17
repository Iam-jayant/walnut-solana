//! Governance roles and two-step admin transfer (Task 4.2, Requirements 4.3
//! and 4.4).
//!
//! The Market stores six role keys: `admin`, `pending_admin`,
//! `upgrade_authority`, `risk_authority`, `treasury_authority`, and
//! `guardian_authority`. Every privileged instruction calls [`authorize`] with
//! the one [`Role`] it requires. A signer matches a role only when it equals
//! that role's stored key exactly; no role stands in for another, and the
//! admin is not an implicit override for the risk, treasury, guardian, or
//! upgrade roles.
//!
//! # Who changes what
//!
//! | Change                        | Signer        | Pure transition             |
//! | ----------------------------- | ------------- | --------------------------- |
//! | `propose_admin(new)`          | admin         | [`propose_admin`]           |
//! | `accept_admin()`              | pending admin | [`accept_admin`]            |
//! | `cancel_admin_transfer()`     | admin         | [`cancel_admin_transfer`]   |
//! | `set_risk_authority(new)`     | admin         | [`set_role`]                |
//! | `set_treasury_authority(new)` | admin         | [`set_role`]                |
//! | `set_guardian_authority(new)` | admin         | [`set_role`]                |
//! | `set_upgrade_authority(new)`  | admin         | [`set_upgrade_authority`]   |
//!
//! The design names the admin as the governance root and does not route role
//! changes through the Task 4.4 timelock, so these setters apply immediately.
//! [`apply_role_change`] is the unauthorized, validated state transition: a
//! timelocked execute instruction can authorize its own proposal and then call
//! it without duplicating any rule.
//!
//! # Multisig keys
//!
//! Role keys are plain `Pubkey`s. On mainnet they are Squads (or equivalent)
//! multisig vault PDAs, which are off the ed25519 curve and sign through CPI
//! with `invoke_signed`. Nothing here requires an on-curve key; a role signs
//! through Anchor's ordinary `Signer` check, which a CPI-signed PDA passes.
//!
//! # Key rules
//!
//! A new role key must be non-zero, must not be one of the Market's own
//! accounts (the Market PDA, the vault authority PDA, and the other accounts
//! linked from the Market), and must differ from its current value. The four
//! separated roles (upgrade, risk, treasury, guardian) must hold four
//! different keys (Requirement 4.4). The admin may share a key with one of
//! them; at initialization the admin and the upgrade authority are both the
//! program upgrade authority.
//!
//! The upgrade authority itself lives in the BPF upgradeable loader. The
//! Market records the expected key, and [`set_upgrade_authority`] accepts a
//! new value only when the loader's program-data account already names it, so
//! the record cannot drift from the loader.

use anchor_lang::prelude::{AnchorDeserialize, AnchorSerialize, Pubkey};

use super::state::{Market, MarketStatus};
use super::validation::{check_upgrade_authority, MarketError, ProgramAccountView};

/// A role a privileged instruction can require.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The governance root: admin transfer and role setters.
    Admin,
    /// The proposed admin; may only accept the transfer.
    PendingAdmin,
    /// The recorded program upgrade authority.
    Upgrade,
    /// The risk-parameter authority.
    Risk,
    /// The treasury authority.
    Treasury,
    /// The guardian authority for expiring emergency pauses.
    Guardian,
}

/// The separately configured authorities a setter changes. Carried in
/// [`crate::events::RoleAuthorityChanged`].
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleKind {
    /// `Market.upgrade_authority`.
    Upgrade,
    /// `Market.risk_authority`.
    Risk,
    /// `Market.treasury_authority`.
    Treasury,
    /// `Market.guardian_authority`.
    Guardian,
}

impl RoleKind {
    /// Every separated role, in a fixed order.
    pub const ALL: [RoleKind; 4] = [
        RoleKind::Upgrade,
        RoleKind::Risk,
        RoleKind::Treasury,
        RoleKind::Guardian,
    ];

    /// The [`Role`] that this setter changes.
    pub fn role(self) -> Role {
        match self {
            RoleKind::Upgrade => Role::Upgrade,
            RoleKind::Risk => Role::Risk,
            RoleKind::Treasury => Role::Treasury,
            RoleKind::Guardian => Role::Guardian,
        }
    }
}

/// Returns the stored key of `role`.
pub fn role_key(market: &Market, role: Role) -> Pubkey {
    match role {
        Role::Admin => market.admin,
        Role::PendingAdmin => market.pending_admin,
        Role::Upgrade => market.upgrade_authority,
        Role::Risk => market.risk_authority,
        Role::Treasury => market.treasury_authority,
        Role::Guardian => market.guardian_authority,
    }
}

fn role_key_mut(market: &mut Market, kind: RoleKind) -> &mut Pubkey {
    match kind {
        RoleKind::Upgrade => &mut market.upgrade_authority,
        RoleKind::Risk => &mut market.risk_authority,
        RoleKind::Treasury => &mut market.treasury_authority,
        RoleKind::Guardian => &mut market.guardian_authority,
    }
}

fn unauthorized(role: Role) -> MarketError {
    match role {
        Role::Admin => MarketError::NotMarketAdmin,
        Role::PendingAdmin => MarketError::NotPendingAdmin,
        Role::Upgrade => MarketError::NotUpgradeAuthority,
        Role::Risk => MarketError::NotRiskAuthority,
        Role::Treasury => MarketError::NotTreasuryAuthority,
        Role::Guardian => MarketError::NotGuardianAuthority,
    }
}

/// Requires `signer` to hold `role` on an active Market.
///
/// The caller must also require `signer` to have signed the transaction
/// (Anchor `Signer`). A zero role key never matches. A missing pending admin
/// fails with `NoPendingAdmin`; any other mismatch fails with the role's own
/// error.
pub fn authorize(market: &Market, role: Role, signer: &Pubkey) -> Result<(), MarketError> {
    if market.status != MarketStatus::Active {
        return Err(MarketError::MarketNotActive);
    }
    let key = role_key(market, role);
    if key == Pubkey::default() {
        return Err(match role {
            Role::PendingAdmin => MarketError::NoPendingAdmin,
            other => unauthorized(other),
        });
    }
    if key != *signer {
        return Err(unauthorized(role));
    }
    Ok(())
}

/// The Market's own accounts. None of them may hold a role: the Market and
/// vault authority PDAs can sign through this program, and the others are
/// program state.
fn protocol_accounts(market: &Market, market_key: &Pubkey) -> [Pubkey; 8] {
    [
        *market_key,
        market.vault_authority,
        market.asset_state,
        market.pool_state,
        market.recovery_position,
        market.wsol_vault,
        market.usdc_vault,
        crate::ID,
    ]
}

/// Requires `key` to be usable as a role key: non-zero and not one of the
/// Market's own accounts. Off-curve keys (multisig PDAs) are accepted.
pub fn validate_role_key(
    market: &Market,
    market_key: &Pubkey,
    key: &Pubkey,
) -> Result<(), MarketError> {
    if *key == Pubkey::default() {
        return Err(MarketError::InvalidAuthority);
    }
    if protocol_accounts(market, market_key).contains(key) {
        return Err(MarketError::InvalidRoleKey);
    }
    Ok(())
}

/// Requires the four separated roles of `market` to be valid, pairwise
/// different keys. `initialize_market` runs it on the Market it is about to
/// write.
pub fn check_market_roles(market: &Market, market_key: &Pubkey) -> Result<(), MarketError> {
    let keys = RoleKind::ALL.map(|kind| role_key(market, kind.role()));
    for (index, key) in keys.iter().enumerate() {
        validate_role_key(market, market_key, key)?;
        if keys[index + 1..].contains(key) {
            return Err(MarketError::RoleKeyNotDistinct);
        }
    }
    validate_role_key(market, market_key, &market.admin)
}

/// Step one of the admin transfer: the admin names `new_admin` as pending.
/// A later proposal overwrites an earlier one. Returns the previous pending
/// admin (all zero when none).
pub fn propose_admin(
    market: &mut Market,
    market_key: &Pubkey,
    signer: &Pubkey,
    new_admin: Pubkey,
) -> Result<Pubkey, MarketError> {
    authorize(market, Role::Admin, signer)?;
    validate_role_key(market, market_key, &new_admin)?;
    if new_admin == market.admin {
        return Err(MarketError::RoleKeyUnchanged);
    }
    let old_pending = market.pending_admin;
    market.pending_admin = new_admin;
    Ok(old_pending)
}

/// Step two of the admin transfer: the pending admin becomes admin and the
/// pending slot is cleared. Returns the previous admin.
pub fn accept_admin(market: &mut Market, signer: &Pubkey) -> Result<Pubkey, MarketError> {
    authorize(market, Role::PendingAdmin, signer)?;
    let old_admin = market.admin;
    market.admin = market.pending_admin;
    market.pending_admin = Pubkey::default();
    Ok(old_admin)
}

/// The admin withdraws a pending transfer. Returns the cleared pending admin.
pub fn cancel_admin_transfer(market: &mut Market, signer: &Pubkey) -> Result<Pubkey, MarketError> {
    authorize(market, Role::Admin, signer)?;
    if market.pending_admin == Pubkey::default() {
        return Err(MarketError::NoPendingAdmin);
    }
    let old_pending = market.pending_admin;
    market.pending_admin = Pubkey::default();
    Ok(old_pending)
}

/// Validates and writes one separated role without checking who asked. Every
/// caller must authorize first ([`set_role`], or a Task 4.4 timelock execute).
/// Returns the previous key.
pub fn apply_role_change(
    market: &mut Market,
    market_key: &Pubkey,
    kind: RoleKind,
    new_key: Pubkey,
) -> Result<Pubkey, MarketError> {
    validate_role_key(market, market_key, &new_key)?;
    let old_key = role_key(market, kind.role());
    if new_key == old_key {
        return Err(MarketError::RoleKeyUnchanged);
    }
    let clashes = RoleKind::ALL
        .iter()
        .filter(|other| **other != kind)
        .any(|other| role_key(market, other.role()) == new_key);
    if clashes {
        return Err(MarketError::RoleKeyNotDistinct);
    }
    *role_key_mut(market, kind) = new_key;
    Ok(old_key)
}

/// The admin sets one separated role. Returns the previous key.
pub fn set_role(
    market: &mut Market,
    market_key: &Pubkey,
    signer: &Pubkey,
    kind: RoleKind,
    new_key: Pubkey,
) -> Result<Pubkey, MarketError> {
    authorize(market, Role::Admin, signer)?;
    apply_role_change(market, market_key, kind, new_key)
}

/// Requires the loader's program-data account to name `expected` as the
/// current upgrade authority of `program`.
pub fn check_loader_upgrade_authority(
    program: &ProgramAccountView<'_>,
    program_data: &ProgramAccountView<'_>,
    expected: &Pubkey,
) -> Result<(), MarketError> {
    check_upgrade_authority(program, program_data, expected).map_err(|error| match error {
        MarketError::InitializerNotUpgradeAuthority => MarketError::UpgradeAuthorityMismatch,
        other => other,
    })
}

/// The admin records a rotated upgrade authority. The loader must already
/// name `new_key` (rotate it in the loader first, for example from the Squads
/// multisig), so the Market record always matches the loader. Returns the
/// previous key.
pub fn set_upgrade_authority(
    market: &mut Market,
    market_key: &Pubkey,
    signer: &Pubkey,
    program: &ProgramAccountView<'_>,
    program_data: &ProgramAccountView<'_>,
    new_key: Pubkey,
) -> Result<Pubkey, MarketError> {
    authorize(market, Role::Admin, signer)?;
    validate_role_key(market, market_key, &new_key)?;
    check_loader_upgrade_authority(program, program_data, &new_key)?;
    apply_role_change(market, market_key, RoleKind::Upgrade, new_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::validation::BPF_LOADER_UPGRADEABLE_ID;

    const MARKET_KEY: Pubkey = Pubkey::new_from_array([0xA0; 32]);
    const PROGRAM_DATA_KEY: Pubkey = Pubkey::new_from_array([0xA1; 32]);

    fn key(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    /// admin 1, upgrade 3, risk 4, treasury 5, guardian 6, no pending admin.
    fn market() -> Market {
        Market {
            layout_version: 1,
            status: MarketStatus::Active,
            bump: 0xFE,
            vault_authority_bump: 0xFD,
            admin: key(1),
            pending_admin: Pubkey::default(),
            upgrade_authority: key(3),
            risk_authority: key(4),
            treasury_authority: key(5),
            guardian_authority: key(6),
            asset_state: key(0x41),
            pool_state: key(0x42),
            recovery_position: key(0x43),
            vault_authority: key(0x44),
            wsol_mint: key(0x45),
            usdc_mint: key(0x46),
            wsol_vault: key(0x47),
            usdc_vault: key(0x48),
            treasury_usdc_account: Pubkey::default(),
            position_count: 0,
            debt_position_count: 0,
            unsettled_recovery_claim_count: 0,
            position_count_cap: 10,
            debt_position_count_cap: 5,
            initialized_slot: 1,
            debt_position_reservations: 0,
            reserved: [0; 120],
        }
    }

    fn program_state() -> Vec<u8> {
        let mut data = 2u32.to_le_bytes().to_vec();
        data.extend(PROGRAM_DATA_KEY.to_bytes());
        data
    }

    fn program_data_state(authority: Option<Pubkey>) -> Vec<u8> {
        let mut data = 3u32.to_le_bytes().to_vec();
        data.extend(7u64.to_le_bytes());
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
        data
    }

    fn views<'a>(
        program: &'a [u8],
        program_data: &'a [u8],
    ) -> (ProgramAccountView<'a>, ProgramAccountView<'a>) {
        (
            ProgramAccountView {
                key: crate::ID,
                owner: BPF_LOADER_UPGRADEABLE_ID,
                data: program,
            },
            ProgramAccountView {
                key: PROGRAM_DATA_KEY,
                owner: BPF_LOADER_UPGRADEABLE_ID,
                data: program_data,
            },
        )
    }

    /// A Squads-style multisig vault PDA: off the ed25519 curve by
    /// construction of `find_program_address`.
    fn squads_vault_pda() -> Pubkey {
        let squads = Pubkey::from_str_const("SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf");
        let multisig = key(0x77);
        let seeds: [&[u8]; 4] = [b"multisig", multisig.as_ref(), b"vault", &[0]];
        let (address, _) = anchor_lang::derive_program_address(&seeds, &squads.to_bytes());
        Pubkey::new_from_array(address)
    }

    // ---- authorize -----------------------------------------------------------

    #[test]
    fn each_role_matches_only_its_own_key() {
        let mut m = market();
        m.pending_admin = key(2);
        let roles = [
            (Role::Admin, key(1), MarketError::NotMarketAdmin),
            (Role::PendingAdmin, key(2), MarketError::NotPendingAdmin),
            (Role::Upgrade, key(3), MarketError::NotUpgradeAuthority),
            (Role::Risk, key(4), MarketError::NotRiskAuthority),
            (Role::Treasury, key(5), MarketError::NotTreasuryAuthority),
            (Role::Guardian, key(6), MarketError::NotGuardianAuthority),
        ];
        for (role, holder, error) in roles {
            assert_eq!(authorize(&m, role, &holder), Ok(()));
            for (_, other, _) in roles.iter().filter(|(r, _, _)| *r != role) {
                assert_eq!(authorize(&m, role, other), Err(error), "{role:?}");
            }
        }
    }

    #[test]
    fn zero_keys_and_inactive_markets_never_authorize() {
        let m = market();
        assert_eq!(
            authorize(&m, Role::PendingAdmin, &Pubkey::default()),
            Err(MarketError::NoPendingAdmin)
        );
        let mut no_guardian = market();
        no_guardian.guardian_authority = Pubkey::default();
        assert_eq!(
            authorize(&no_guardian, Role::Guardian, &Pubkey::default()),
            Err(MarketError::NotGuardianAuthority)
        );
        let inactive = Market {
            status: MarketStatus::Uninitialized,
            ..market()
        };
        assert_eq!(
            authorize(&inactive, Role::Admin, &key(1)),
            Err(MarketError::MarketNotActive)
        );
    }

    // ---- two-step admin transfer -------------------------------------------

    #[test]
    fn propose_then_accept_moves_admin_and_clears_pending() {
        let mut m = market();
        assert_eq!(
            propose_admin(&mut m, &MARKET_KEY, &key(1), key(9)),
            Ok(Pubkey::default())
        );
        assert_eq!(m.pending_admin, key(9));
        assert_eq!(m.admin, key(1));
        assert_eq!(
            accept_admin(&mut m, &key(1)),
            Err(MarketError::NotPendingAdmin)
        );
        assert_eq!(accept_admin(&mut m, &key(9)), Ok(key(1)));
        assert_eq!(m.admin, key(9));
        assert_eq!(m.pending_admin, Pubkey::default());
        // The old admin lost every admin power.
        assert_eq!(
            propose_admin(&mut m, &MARKET_KEY, &key(1), key(8)),
            Err(MarketError::NotMarketAdmin)
        );
        assert_eq!(
            accept_admin(&mut m, &key(9)),
            Err(MarketError::NoPendingAdmin)
        );
    }

    #[test]
    fn re_propose_overwrites_and_cancel_clears() {
        let mut m = market();
        propose_admin(&mut m, &MARKET_KEY, &key(1), key(9)).unwrap();
        assert_eq!(
            propose_admin(&mut m, &MARKET_KEY, &key(1), key(8)),
            Ok(key(9))
        );
        assert_eq!(
            accept_admin(&mut m, &key(9)),
            Err(MarketError::NotPendingAdmin)
        );
        assert_eq!(
            cancel_admin_transfer(&mut m, &key(8)),
            Err(MarketError::NotMarketAdmin)
        );
        assert_eq!(cancel_admin_transfer(&mut m, &key(1)), Ok(key(8)));
        assert_eq!(m.pending_admin, Pubkey::default());
        assert_eq!(
            accept_admin(&mut m, &key(8)),
            Err(MarketError::NoPendingAdmin)
        );
        assert_eq!(
            cancel_admin_transfer(&mut m, &key(1)),
            Err(MarketError::NoPendingAdmin)
        );
        assert_eq!(m.admin, key(1));
    }

    #[test]
    fn proposals_reject_zero_protocol_and_unchanged_keys() {
        let mut m = market();
        let before = market();
        for (bad, error) in [
            (Pubkey::default(), MarketError::InvalidAuthority),
            (MARKET_KEY, MarketError::InvalidRoleKey),
            (m.vault_authority, MarketError::InvalidRoleKey),
            (crate::ID, MarketError::InvalidRoleKey),
            (key(1), MarketError::RoleKeyUnchanged),
        ] {
            assert_eq!(propose_admin(&mut m, &MARKET_KEY, &key(1), bad), Err(error));
            assert_eq!(m, before);
        }
    }

    // ---- role setters ----------------------------------------------------------

    #[test]
    fn the_admin_sets_each_role_and_only_the_admin() {
        for kind in [RoleKind::Risk, RoleKind::Treasury, RoleKind::Guardian] {
            let mut m = market();
            // The role's own holder cannot rotate itself, and no other role
            // stands in for the admin.
            for signer in [key(3), key(4), key(5), key(6), key(9)] {
                assert_eq!(
                    set_role(&mut m, &MARKET_KEY, &signer, kind, key(20)),
                    Err(MarketError::NotMarketAdmin)
                );
            }
            let old = role_key(&m, kind.role());
            assert_eq!(
                set_role(&mut m, &MARKET_KEY, &key(1), kind, key(20)),
                Ok(old)
            );
            assert_eq!(role_key(&m, kind.role()), key(20));
            assert_eq!(authorize(&m, kind.role(), &key(20)), Ok(()));
            assert!(authorize(&m, kind.role(), &old).is_err());
        }
    }

    #[test]
    fn setters_reject_invalid_unchanged_and_shared_keys() {
        let mut m = market();
        let before = market();
        let cases = [
            (
                RoleKind::Risk,
                Pubkey::default(),
                MarketError::InvalidAuthority,
            ),
            (RoleKind::Risk, MARKET_KEY, MarketError::InvalidRoleKey),
            (RoleKind::Treasury, key(0x44), MarketError::InvalidRoleKey),
            (RoleKind::Guardian, key(0x48), MarketError::InvalidRoleKey),
            (RoleKind::Risk, key(4), MarketError::RoleKeyUnchanged),
            (RoleKind::Risk, key(5), MarketError::RoleKeyNotDistinct),
            (RoleKind::Guardian, key(3), MarketError::RoleKeyNotDistinct),
        ];
        for (kind, bad, error) in cases {
            assert_eq!(
                set_role(&mut m, &MARKET_KEY, &key(1), kind, bad),
                Err(error)
            );
            assert_eq!(m, before);
        }
        // The admin's key may also hold a separated role.
        assert_eq!(
            set_role(&mut m, &MARKET_KEY, &key(1), RoleKind::Treasury, key(1)),
            Ok(key(5))
        );
    }

    #[test]
    fn an_off_curve_multisig_pda_is_accepted_as_every_role() {
        let vault = squads_vault_pda();
        let mut m = market();
        assert_eq!(
            set_role(&mut m, &MARKET_KEY, &key(1), RoleKind::Risk, vault),
            Ok(key(4))
        );
        assert_eq!(authorize(&m, Role::Risk, &vault), Ok(()));
        propose_admin(&mut m, &MARKET_KEY, &key(1), vault).unwrap();
        assert_eq!(accept_admin(&mut m, &vault), Ok(key(1)));
        assert_eq!(authorize(&m, Role::Admin, &vault), Ok(()));
        assert_eq!(
            set_role(&mut m, &MARKET_KEY, &vault, RoleKind::Guardian, key(30)),
            Ok(key(6))
        );
        let program = program_state();
        let data = program_data_state(Some(squads_vault_pda()));
        let (p, d) = views(&program, &data);
        let new_upgrade = squads_vault_pda();
        // Risk already holds the vault, so the upgrade role must differ.
        assert_eq!(
            set_upgrade_authority(&mut m, &MARKET_KEY, &vault, &p, &d, new_upgrade),
            Err(MarketError::RoleKeyNotDistinct)
        );
        set_role(&mut m, &MARKET_KEY, &vault, RoleKind::Risk, key(31)).unwrap();
        assert_eq!(
            set_upgrade_authority(&mut m, &MARKET_KEY, &vault, &p, &d, new_upgrade),
            Ok(key(3))
        );
        assert_eq!(m.upgrade_authority, new_upgrade);
    }

    #[test]
    fn upgrade_authority_must_match_the_loader() {
        let program = program_state();
        let mut m = market();
        let current = program_data_state(Some(key(3)));
        let (p, d) = views(&program, &current);
        assert_eq!(
            set_upgrade_authority(&mut m, &MARKET_KEY, &key(1), &p, &d, key(3)),
            Err(MarketError::RoleKeyUnchanged)
        );
        assert_eq!(
            set_upgrade_authority(&mut m, &MARKET_KEY, &key(1), &p, &d, key(10)),
            Err(MarketError::UpgradeAuthorityMismatch)
        );
        let frozen = program_data_state(None);
        let (p, d) = views(&program, &frozen);
        assert_eq!(
            set_upgrade_authority(&mut m, &MARKET_KEY, &key(1), &p, &d, key(10)),
            Err(MarketError::UpgradeAuthorityMismatch)
        );
        let rotated = program_data_state(Some(key(10)));
        let (p, d) = views(&program, &rotated);
        assert_eq!(
            set_upgrade_authority(&mut m, &MARKET_KEY, &key(3), &p, &d, key(10)),
            Err(MarketError::NotMarketAdmin)
        );
        let wrong_data = ProgramAccountView {
            key: key(0xA2),
            ..d
        };
        assert_eq!(
            set_upgrade_authority(&mut m, &MARKET_KEY, &key(1), &p, &wrong_data, key(10)),
            Err(MarketError::InvalidProgramDataAccount)
        );
        assert_eq!(m, market());
        assert_eq!(
            set_upgrade_authority(&mut m, &MARKET_KEY, &key(1), &p, &d, key(10)),
            Ok(key(3))
        );
        assert_eq!(m.upgrade_authority, key(10));
    }

    #[test]
    fn market_role_check_requires_four_distinct_valid_keys() {
        assert_eq!(check_market_roles(&market(), &MARKET_KEY), Ok(()));
        let shared = Market {
            guardian_authority: key(4),
            ..market()
        };
        assert_eq!(
            check_market_roles(&shared, &MARKET_KEY),
            Err(MarketError::RoleKeyNotDistinct)
        );
        let pda = Market {
            treasury_authority: key(0x44),
            ..market()
        };
        assert_eq!(
            check_market_roles(&pda, &MARKET_KEY),
            Err(MarketError::InvalidRoleKey)
        );
        // Admin may equal the upgrade authority (the initializer is both).
        let initial = Market {
            admin: key(3),
            ..market()
        };
        assert_eq!(check_market_roles(&initial, &MARKET_KEY), Ok(()));
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

    /// The instructions that change roles, as pure calls.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        Propose,
        Accept,
        Cancel,
        Set(RoleKind),
        SetUpgrade,
    }

    const OPS: [Op; 7] = [
        Op::Propose,
        Op::Accept,
        Op::Cancel,
        Op::Set(RoleKind::Risk),
        Op::Set(RoleKind::Treasury),
        Op::Set(RoleKind::Guardian),
        Op::SetUpgrade,
    ];

    fn run(
        m: &mut Market,
        op: Op,
        signer: &Pubkey,
        new_key: Pubkey,
    ) -> Result<Pubkey, MarketError> {
        let program = program_state();
        let data = program_data_state(Some(new_key));
        let (p, d) = views(&program, &data);
        match op {
            Op::Propose => propose_admin(m, &MARKET_KEY, signer, new_key),
            Op::Accept => accept_admin(m, signer),
            Op::Cancel => cancel_admin_transfer(m, signer),
            Op::Set(kind) => set_role(m, &MARKET_KEY, signer, kind, new_key),
            Op::SetUpgrade => set_upgrade_authority(m, &MARKET_KEY, signer, &p, &d, new_key),
        }
    }

    /// **Validates: Requirements 4.3, 4.4** - for every role-changing
    /// instruction, a signer that does not hold the required role (admin, or
    /// pending admin for accept) is always rejected with that role's error and
    /// leaves the Market unchanged, whatever the other roles hold.
    #[test]
    fn prop_unauthorized_signer_always_rejected() {
        let mut rng = Lcg(0x5EED_4201);
        let people: Vec<Pubkey> = (1..=8).map(key).collect();
        let mut rejected = 0;
        for _ in 0..5_000 {
            let mut m = market();
            m.admin = rng.pick(&people);
            m.pending_admin = if rng.sample().is_multiple_of(3) {
                Pubkey::default()
            } else {
                rng.pick(&people)
            };
            let op = rng.pick(&OPS);
            let signer = rng.pick(&people);
            let new_key = key(0x30 + (rng.sample() % 4) as u8);
            let required = match op {
                Op::Accept => Role::PendingAdmin,
                _ => Role::Admin,
            };
            let holder = role_key(&m, required);
            if holder == signer {
                continue;
            }
            let before = m.clone();
            let expected = match (required, holder == Pubkey::default()) {
                (Role::PendingAdmin, true) => MarketError::NoPendingAdmin,
                (Role::PendingAdmin, false) => MarketError::NotPendingAdmin,
                _ => MarketError::NotMarketAdmin,
            };
            assert_eq!(run(&mut m, op, &signer, new_key), Err(expected), "{op:?}");
            assert_eq!(m, before);
            rejected += 1;
        }
        assert!(rejected > 1_000);
    }

    /// **Validates: Requirements 4.3** - over random sequences of propose,
    /// accept, and cancel by random signers, the Market follows the two-step
    /// model: only the current pending admin can accept, accepting moves the
    /// pending key into admin and clears pending, cancel clears pending, and a
    /// new proposal overwrites the old one. The admin changes only on accept.
    #[test]
    fn prop_two_step_transfer_state_machine() {
        let mut rng = Lcg(0x5EED_4202);
        let people: Vec<Pubkey> = (1..=5).map(key).collect();
        let mut accepted = 0;
        for _ in 0..500 {
            let mut m = market();
            let (mut admin, mut pending) = (m.admin, Pubkey::default());
            for _ in 0..20 {
                let signer = rng.pick(&people);
                let target = rng.pick(&people);
                match rng.sample() % 3 {
                    0 => {
                        let result = propose_admin(&mut m, &MARKET_KEY, &signer, target);
                        if signer != admin {
                            assert_eq!(result, Err(MarketError::NotMarketAdmin));
                        } else if target == admin {
                            assert_eq!(result, Err(MarketError::RoleKeyUnchanged));
                        } else {
                            assert_eq!(result, Ok(pending));
                            pending = target;
                        }
                    }
                    1 => {
                        let result = accept_admin(&mut m, &signer);
                        if pending == Pubkey::default() {
                            assert_eq!(result, Err(MarketError::NoPendingAdmin));
                        } else if signer != pending {
                            assert_eq!(result, Err(MarketError::NotPendingAdmin));
                        } else {
                            assert_eq!(result, Ok(admin));
                            admin = pending;
                            pending = Pubkey::default();
                            accepted += 1;
                        }
                    }
                    _ => {
                        let result = cancel_admin_transfer(&mut m, &signer);
                        if signer != admin {
                            assert_eq!(result, Err(MarketError::NotMarketAdmin));
                        } else if pending == Pubkey::default() {
                            assert_eq!(result, Err(MarketError::NoPendingAdmin));
                        } else {
                            assert_eq!(result, Ok(pending));
                            pending = Pubkey::default();
                        }
                    }
                }
                assert_eq!((m.admin, m.pending_admin), (admin, pending));
                // Other roles never move during an admin transfer.
                assert_eq!(m.risk_authority, key(4));
                assert_eq!(m.upgrade_authority, key(3));
            }
        }
        assert!(accepted > 100, "the generator must reach accept");
    }

    /// **Validates: Requirements 4.4** - zero keys and the Market's own
    /// accounts (Market PDA, vault authority PDA, linked state and vaults, the
    /// program id) are rejected by every role-changing instruction, even for
    /// the authorized signer, and leave the Market unchanged; any other fresh
    /// key, including an off-curve PDA, is accepted.
    #[test]
    fn prop_zero_and_protocol_keys_rejected() {
        let mut rng = Lcg(0x5EED_4203);
        let base = market();
        let bad = protocol_accounts(&base, &MARKET_KEY);
        let ops = [
            Op::Propose,
            Op::Set(RoleKind::Risk),
            Op::Set(RoleKind::Treasury),
            Op::Set(RoleKind::Guardian),
            Op::SetUpgrade,
        ];
        for _ in 0..2_000 {
            let op = rng.pick(&ops);
            let mut m = market();
            let zero = rng.sample().is_multiple_of(4);
            let new_key = if zero {
                Pubkey::default()
            } else {
                rng.pick(&bad)
            };
            let expected = if zero {
                MarketError::InvalidAuthority
            } else {
                MarketError::InvalidRoleKey
            };
            assert_eq!(run(&mut m, op, &key(1), new_key), Err(expected), "{op:?}");
            assert_eq!(m, base);
            let fresh = if rng.sample().is_multiple_of(2) {
                squads_vault_pda()
            } else {
                key(0x60 + (rng.sample() % 8) as u8)
            };
            assert!(run(&mut m, op, &key(1), fresh).is_ok(), "{op:?}");
        }
    }
}
