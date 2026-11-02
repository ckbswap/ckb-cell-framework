#![no_std]
extern crate alloc;
pub mod cells;
pub mod state;
pub use state::*;

use cells::{PoolArgs, PoolCells};
use ckb_contract_core::{
    args::RoleArgs,
    check_flow, check_supply,
    layout::{ReserveLayout, StateLayout, TransactionLayout},
    token::{AmountEncoding, PlainXudtAsset, XUDT_CODE_HASH},
    Error, Hash, Result, CKB,
};
use ckb_contract_onchain::{
    context::Context, script, state as state_cells, syscall, token, Source,
};
use ckb_std::{
    ckb_types::{packed::Script, prelude::*},
    high_level::load_script,
};

pub const STATE_ROLE: u8 = 0;
pub const LOCK_ROLE: u8 = 1;
pub const RESERVE_X_ROLE: u8 = 2;
pub const RESERVE_Y_ROLE: u8 = 3;
/// xUDT owner flags: disable input-Lock authorization, enable input/output-Type
/// authorization. Low 29 bits are zero, so no extension scripts are required.
pub const LP_OWNER_FLAGS: u32 = 0xe000_0000;
pub const STATE_CAPACITY: u64 = 400 * CKB;
pub const RESERVE_CAPACITY: u64 = 200 * CKB;
/// Application layout, shared with off-chain ContextTx builders.
pub const LAYOUT: TransactionLayout<'static> = TransactionLayout {
    state: StateLayout {
        role: STATE_ROLE,
        lock_role: LOCK_ROLE,
        capacity: STATE_CAPACITY,
        output: 0,
    },
    reserves: &[
        ReserveLayout {
            role: RESERVE_X_ROLE,
            capacity: RESERVE_CAPACITY,
            output: 1,
        },
        ReserveLayout {
            role: RESERVE_Y_ROLE,
            capacity: RESERVE_CAPACITY,
            output: 2,
        },
    ],
};

pub fn verify() -> Result<()> {
    let current = syscall(load_script())?;
    let raw = current.args().raw_data();
    let role = RoleArgs::decode(&raw)?;
    role.require_payload_len(0)?;
    let args = PoolArgs::new(&current, &role.id);
    match role.role {
        STATE_ROLE => verify_pool(Context::<PoolCells>::load(&args)?, &args),
        LOCK_ROLE => state_cells::verify_state_lock(&args.state_type),
        RESERVE_X_ROLE | RESERVE_Y_ROLE => {
            state_cells::require_state_input(&args.state_type).map(|_| ())
        }
        _ => Err(Error::Encoding),
    }
}

fn asset_type(asset: PlainXudtAsset) -> Script {
    script::plain_xudt(&asset.owner)
        .as_builder()
        .args(asset.args().pack())
        .build()
}

/// Official xUDT LP, owned by this pool's full State Type hash. The output
/// Type authorizes genesis; subsequent State transitions authorize issuance.
pub fn lp_type(state_type: &Script) -> Script {
    let mut args = script::script_hash(state_type).to_vec();
    args.extend_from_slice(&LP_OWNER_FLAGS.to_le_bytes());
    Script::new_builder()
        .code_hash(XUDT_CODE_HASH.pack())
        .hash_type(2u8)
        .args(args.pack())
        .build()
}

fn party_flow(asset: &Hash, actor: &Hash, spent: u128, received: u128) -> Result<()> {
    check_flow(
        token::total(asset, Source::Input, Some(actor), AmountEncoding::Exact16)?,
        token::total(asset, Source::Output, Some(actor), AmountEncoding::Exact16)?,
        spent,
        received,
    )
}

pub fn verify_pool(ctx: Context<PoolCells>, args: &PoolArgs) -> Result<()> {
    cells::verify_protocol(&ctx, args)?;
    verify_settlement(&ctx, args)
}

fn verify_settlement(ctx: &Context<PoolCells>, args: &PoolArgs) -> Result<()> {
    let flow = cells::settlement(&ctx.cells)?;
    let pool = ctx.cells.after.data();
    for (asset, spent, received) in [
        (pool.asset_x, flow.spent_x, flow.received_x),
        (pool.asset_y, flow.spent_y, flow.received_y),
    ] {
        let hash = script::script_hash(&asset_type(asset));
        token::require_supply_change(&hash, 0, 0)?;
        party_flow(&hash, &flow.actor, spent, received)?;
    }
    let lp = script::script_hash(&args.lp_type);
    check_supply(
        token::total(&lp, Source::Input, None, AmountEncoding::Exact16)?,
        token::total(&lp, Source::Output, None, AmountEncoding::Exact16)?,
        flow.minted,
        flow.burned,
    )?;
    if flow.minted != 0 || flow.burned != 0 {
        party_flow(&lp, &flow.actor, flow.burned, flow.minted)?;
    }
    Ok(())
}
