#![no_std]
extern crate alloc;
pub mod state;
pub use state::*;
// Deployment roles and Cell layout policy for this example.
use ckb_contract_core::CKB;

pub const STATE_ROLE: u8 = 0;
pub const LOCK_ROLE: u8 = 1;
pub const RESERVE_ROLE: u8 = 2;
pub const STATE_CAPACITY: u64 = 350 * CKB;
pub const RESERVE_CAPACITY: u64 = 200 * CKB;

pub mod cells;

use ckb_contract_core::{
    args::RoleArgs,
    check_flow,
    token::{AmountEncoding, PlainXudtAsset},
    Error, Hash, Result,
};
use ckb_contract_onchain::{
    cells as cell_checks, context::Context, script, state as state_cells, syscall, token, Source,
};
use ckb_std::{
    ckb_types::{packed::Script, prelude::*},
    high_level::*,
};

use self::cells::{OrderArgs, OrderCells};

pub fn verify() -> Result<()> {
    let current = syscall(load_script())?;
    let raw_args = current.args().raw_data();
    let args = RoleArgs::decode(&raw_args)?;
    args.require_payload_len(0)?;
    let state_type = script::with_role(&current, STATE_ROLE, &args.id, &[]);
    match args.role {
        STATE_ROLE => {
            let args = OrderArgs::new(&current, &args.id);
            verify_order(Context::<OrderCells>::load(&args)?, &args)
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

fn verify_party_flow(asset: &Hash, party: &Hash, spent: u128, received: u128) -> Result<()> {
    check_flow(
        token::total(asset, Source::Input, Some(party), AmountEncoding::Exact16)?,
        token::total(asset, Source::Output, Some(party), AmountEncoding::Exact16)?,
        spent,
        received,
    )
}

/// Check both assets' supply and the maker/taker net flows. Maker-owned inputs
/// cannot disguise a missing quote payment; taker inputs cannot disguise a sale.
fn verify_asset_flow(
    order: &State,
    maker_spent: u128,
    settlement: Option<&Settlement>,
) -> Result<()> {
    let sell = script::script_hash(&asset_type(order.sell_asset));
    let buy = script::script_hash(&asset_type(order.buy_asset));
    token::require_supply_change(&sell, 0, 0)?;
    token::require_supply_change(&buy, 0, 0)?;
    let maker_refund = settlement
        .filter(|flow| flow.sell_recipient == order.maker)
        .map_or(0, |flow| flow.sell_amount);
    let buy_payment = settlement.map_or(0, |flow| flow.buy_amount);
    verify_party_flow(&sell, &order.maker, maker_spent, maker_refund)?;
    verify_party_flow(&buy, &order.maker, 0, buy_payment)?;
    if let Some(flow) = settlement.filter(|flow| flow.actor != order.maker) {
        verify_party_flow(&sell, &flow.actor, 0, flow.sell_amount)?;
        verify_party_flow(&buy, &flow.actor, flow.buy_amount, 0)?;
    }
    Ok(())
}

pub fn verify_order(ctx: Context<OrderCells>, args: &OrderArgs) -> Result<()> {
    self::cells::verify_protocol(&ctx, args)?;
    verify_settlement(&ctx)
}

fn verify_settlement(ctx: &Context<OrderCells>) -> Result<()> {
    match (&ctx.cells.before, &ctx.cells.after) {
        (None, Some(after)) => verify_asset_flow(after.data(), after.data().sell_total, None),
        (Some(before), Some(_)) => {
            let settlement = before
                .data()
                .settlement(&cell_checks::action::<Action>(before.index())?)?;
            verify_asset_flow(before.data(), 0, Some(&settlement))
        }
        (Some(before), None) => {
            verify_asset_flow(before.data(), 0, None)?;
            cell_checks::require_payment(
                &before.data().maker,
                STATE_CAPACITY + RESERVE_CAPACITY,
                cell_checks::transaction_fee()?,
            )
        }
        _ => Err(Error::Structure),
    }
}
