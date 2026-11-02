//! Two-party escrow validation. Read the entry point, lifecycle branches, then reserve and flow checks.
//!
//! Creation: the buyer funds the escrow, producing a Funded State and its Reserve.
//! Release: the buyer authorizes payment to the seller. Refund: the seller authorizes payment to the buyer.
//! Settlement retains the State and a zero-token Reserve; the buyer recovers their CKB capacity on close.
//! The script validates the transaction supplied by the client; it does not create or modify outputs.
#![no_std]
extern crate alloc;
pub mod cells;
pub mod state;
pub use state::*;
// Capacity is measured in shannons; CKB converts whole CKB amounts to capacity values.
use ckb_contract_core::CKB;

// All three roles share one binary and are distinguished by the role and instance ID in args.
// The State Type validates business rules; the public State and Reserve Locks depend on it.
pub const STATE_ROLE: u8 = 0;
pub const LOCK_ROLE: u8 = 1;
pub const RESERVE_ROLE: u8 = 2;
// State is fixed at output 0 and Reserve at output 1, each with a fixed CKB capacity.
// The token amount is stored in Reserve data and is separate from its CKB capacity.
pub const STATE_CAPACITY: u64 = 350 * CKB;
pub const RESERVE_CAPACITY: u64 = 200 * CKB;

use ckb_contract_core::{args::RoleArgs, check_flow, token::AmountEncoding, Error, Hash, Result};
use ckb_contract_onchain::{
    cells as cell_checks, context::Context, script, state as state_cells, syscall, token, Source,
};
use ckb_std::{
    ckb_types::{packed::Script, prelude::*},
    high_level::*,
};

/// Parse the current Script args and dispatch to the validator for its role.
use self::cells::{EscrowArgs, EscrowCells};

pub fn verify() -> Result<()> {
    let current = syscall(load_script())?;
    let raw_args = current.args().raw_data();
    // Args are role:u8 || entity_id:bytes32; this example accepts no additional payload.
    let args = RoleArgs::decode(&raw_args)?;
    args.require_payload_len(0)?;
    // Preserve the code identity and instance ID, changing only the role to derive the full State Type.
    let state_type = script::with_role(&current, STATE_ROLE, &args.id, &[]);
    match args.role {
        STATE_ROLE => {
            let args = EscrowArgs::new(&current, &args.id);
            verify_order(Context::<EscrowCells>::load(&args)?, &args)
        }
        // The public State Lock requires the matching Type on its input; that Type checks business rules.
        LOCK_ROLE => state_cells::verify_state_lock(&state_type),
        // A Reserve can be spent only in a transaction that also consumes its matching State.
        RESERVE_ROLE => state_cells::require_state_input(&state_type).map(|_| ()),
        _ => Err(Error::Encoding),
    }
}

/// Reconstruct the full asset Type, preserving its original 32- or 36-byte args and identity.
fn asset_type(order: &State) -> Script {
    script::plain_xudt(&order.asset.owner)
        .as_builder()
        .args(order.asset.args().pack())
        .build()
}

/// Check each party's net token flow across all matching Cells, including their change.
/// Creation uses buyer_spent=amount and recipient=None; settlement uses zero spent and a recipient.
/// Require outputs + expected spending = inputs + expected receipts to reject underpayment or a substitute payer.
fn verify_asset_flow(order: &State, buyer_spent: u128, recipient: Option<&Hash>) -> Result<()> {
    let hash = script::script_hash(&asset_type(order));
    // Neither minting nor burning is allowed; escrow transfers must preserve this asset's supply.
    token::require_supply_change(&hash, 0, 0)?;
    for party in [&order.buyer, &order.seller] {
        check_flow(
            token::total(&hash, Source::Input, Some(party), AmountEncoding::Exact16)?,
            token::total(&hash, Source::Output, Some(party), AmountEncoding::Exact16)?,
            if *party == order.buyer {
                buyer_spent
            } else {
                0
            },
            if recipient == Some(party) {
                order.amount
            } else {
                0
            },
        )?;
    }
    Ok(())
}

/// Load fields, verify the protocol, then check actual settlement and refunds.
pub fn verify_order(ctx: Context<EscrowCells>, args: &EscrowArgs) -> Result<()> {
    self::cells::verify_protocol(&ctx, args)?;
    verify_settlement(&ctx)
}

fn verify_settlement(ctx: &Context<EscrowCells>) -> Result<()> {
    match (&ctx.cells.before, &ctx.cells.after) {
        (None, Some(after)) => verify_asset_flow(after.data(), after.data().amount, None),
        (Some(before), Some(_)) => {
            let settlement = before
                .data()
                .settlement(&cell_checks::action::<Action>(before.index())?)?;
            verify_asset_flow(before.data(), 0, Some(&settlement.recipient))
        }
        (Some(before), None) => {
            verify_asset_flow(before.data(), 0, None)?;
            cell_checks::require_payment(
                &before.data().buyer,
                STATE_CAPACITY + RESERVE_CAPACITY,
                cell_checks::transaction_fee()?,
            )
        }
        _ => Err(Error::Structure),
    }
}
