#![no_std]
extern crate alloc;
pub mod cells;
pub mod state;
pub use state::*;
// Deployment roles and Cell layout policy for this example.
use ckb_contract_core::CKB;

pub const STATE_ROLE: u8 = 0;
pub const LOCK_ROLE: u8 = 1;
pub const RESERVE_ROLE: u8 = 2;
pub const STATE_CAPACITY: u64 = 300 * CKB;
pub const RESERVE_CAPACITY: u64 = 200 * CKB;

use ckb_contract_core::{
    args::RoleArgs,
    check_flow,
    token::{AmountEncoding, PlainXudtAsset},
    Error, Result,
};
use ckb_contract_onchain::{
    cells as cell_checks, context::Context, script, state as state_cells, syscall, token, Source,
};
use ckb_std::{
    ckb_types::{packed::Script, prelude::*},
    high_level::*,
};

use self::cells::{VaultArgs, VaultCells};

pub fn verify() -> Result<()> {
    let current = syscall(load_script())?;
    let raw_args = current.args().raw_data();
    let args = RoleArgs::decode(&raw_args)?;
    args.require_payload_len(0)?;
    let state_type = script::with_role(&current, STATE_ROLE, &args.id, &[]);
    match args.role {
        STATE_ROLE => {
            let args = VaultArgs::new(&current, &args.id);
            verify_account(Context::<VaultCells>::load(&args)?, &args)
        }
        LOCK_ROLE => state_cells::verify_state_lock(&state_type),
        RESERVE_ROLE => state_cells::require_state_input(&state_type).map(|_| ()),
        _ => Err(Error::Encoding),
    }
}

fn asset_type(asset: PlainXudtAsset) -> Script {
    script::plain_xudt(&asset.owner)
        .as_builder()
        .args(asset.args().pack())
        .build()
}

/// Full asset conservation and the owner's exact net payment. Principal cannot
/// be paid by another account or redirected to a third-party recipient.
fn verify_asset_flow(account: &State, spent: u128, received: u128) -> Result<()> {
    let hash = script::script_hash(&asset_type(account.asset));
    let total = |source, owner| token::total(&hash, source, owner, AmountEncoding::Exact16);
    token::require_supply_change(&hash, 0, 0)?;
    check_flow(
        total(Source::Input, Some(&account.owner))?,
        total(Source::Output, Some(&account.owner))?,
        spent,
        received,
    )
}

pub fn verify_account(ctx: Context<VaultCells>, args: &VaultArgs) -> Result<()> {
    self::cells::verify_protocol(&ctx, args)?;
    verify_settlement(&ctx)
}

fn verify_settlement(ctx: &Context<VaultCells>) -> Result<()> {
    match (&ctx.cells.before, &ctx.cells.after) {
        (None, Some(after)) => verify_asset_flow(after.data(), 0, 0),
        (Some(before), Some(_)) => match cell_checks::action::<Action>(before.index())? {
            Action::Deposit(amount) => verify_asset_flow(before.data(), amount, 0),
            Action::Withdraw(amount) => verify_asset_flow(before.data(), 0, amount),
            Action::Close => Err(Error::Transition),
        },
        (Some(before), None) => {
            verify_asset_flow(before.data(), 0, 0)?;
            cell_checks::require_payment(
                &before.data().owner,
                STATE_CAPACITY + RESERVE_CAPACITY,
                cell_checks::transaction_fee()?,
            )
        }
        _ => Err(Error::Structure),
    }
}
