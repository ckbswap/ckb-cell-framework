//! Cell selection and explicit pool protocol checks.
use crate::{
    Action, Settlement, State, LOCK_ROLE, RESERVE_CAPACITY, RESERVE_X_ROLE, RESERVE_Y_ROLE,
    STATE_CAPACITY, STATE_ROLE,
};
use ckb_contract_core::{
    token::{decode_amount, AmountEncoding},
    Error, Hash, Result, StateTransition,
};
use ckb_contract_onchain::{
    cells::{self, ScriptField},
    context::{Cell, Cells, Context, InputAuthority, RawData},
    script, state as state_cells, syscall, token, Source,
};
use ckb_std::{
    ckb_types::{packed::Script, prelude::*},
    high_level::load_input,
};

pub struct PoolArgs {
    pub state_type: Script,
    pub state_lock: Script,
    pub reserve_x_lock: Script,
    pub reserve_y_lock: Script,
    pub lp_type: Script,
    pub id: Hash,
}

impl PoolArgs {
    pub fn new(base: &Script, id: &Hash) -> Self {
        let role = |role| script::with_role(base, role, id, &[]);
        let state_type = role(STATE_ROLE);
        let lp_type = crate::lp_type(&state_type);
        Self {
            state_type,
            state_lock: role(LOCK_ROLE),
            reserve_x_lock: role(RESERVE_X_ROLE),
            reserve_y_lock: role(RESERVE_Y_ROLE),
            lp_type,
            id: *id,
        }
    }
}

#[derive(Cells)]
#[cells(args = PoolArgs)]
pub struct PoolCells {
    #[cell(
        source = Source::Input,
        type_script = Some(&args.state_type),
        capacity = STATE_CAPACITY,
        constraint = before.as_ref().is_none_or(|cell| cell.lock() == &args.state_lock)
    )]
    pub before: Option<Cell<State>>,

    #[cell(
        source = Source::Output,
        type_script = Some(&args.state_type),
        capacity = STATE_CAPACITY,
        constraint = after.index() == 0 && after.lock() == &args.state_lock
    )]
    pub after: Cell<State>,

    #[cell(
        source = Source::Input,
        lock = &args.reserve_x_lock,
        capacity = RESERVE_CAPACITY
    )]
    pub x_before: Option<Cell<RawData>>,

    #[cell(
        source = Source::Output,
        lock = &args.reserve_x_lock,
        capacity = RESERVE_CAPACITY,
        constraint = x_after.index() == 1
    )]
    pub x_after: Cell<RawData>,

    #[cell(
        source = Source::Input,
        lock = &args.reserve_y_lock,
        capacity = RESERVE_CAPACITY
    )]
    pub y_before: Option<Cell<RawData>>,

    #[cell(
        source = Source::Output,
        lock = &args.reserve_y_lock,
        capacity = RESERVE_CAPACITY,
        constraint = y_after.index() == 2
    )]
    pub y_after: Cell<RawData>,
}

pub fn settlement(pool: &PoolCells) -> Result<Settlement> {
    match &pool.before {
        Some(before) => before
            .data()
            .settlement(&cells::action::<Action>(before.index())?),
        None => pool.after.data().creation_settlement(),
    }
}

pub fn verify_protocol(ctx: &Context<PoolCells>, args: &PoolArgs) -> Result<()> {
    let pool = &ctx.cells;
    // Exempt only this pool's full LP xUDT Type hash; settlement checks the exact
    // mint amount. Reject net issuance of all other official xUDT/sUDT assets so
    // public pool Locks cannot be used as minting authorization.
    token::reject_standard_mint(&[script::script_hash(&args.lp_type)])?;
    // Reject this pool's full State Type script as an input Lock: state validation
    // does not authorize wallet spending and must not be used to unlock assets.
    state_cells::reject_type_as_input_lock(&args.state_type)?;
    // Also reject this pool's full LP Type script as an input Lock: xUDT supply
    // checks do not authorize holder spending. LP Cells belong under wallet Locks.
    state_cells::reject_type_as_input_lock(&args.lp_type)?;
    if pool.before.is_some() != pool.x_before.is_some()
        || pool.before.is_some() != pool.y_before.is_some()
    {
        return Err(Error::Structure);
    }
    // Creation consumes no pool Cells; updates consume one State and both reserves.
    // Both operations must produce exactly one State and one reserve per asset.
    for (source, expected) in [
        (Source::Input, usize::from(pool.before.is_some())),
        (Source::Output, 1),
    ] {
        // Count the State Type and the State/Reserve Locks in their respective
        // fields. Counts cover this deployment's code across all pool IDs, so
        // duplicate roles or extra Cells from another pool are rejected.
        for (role, field) in [
            (STATE_ROLE, ScriptField::Type),
            (LOCK_ROLE, ScriptField::Lock),
            (RESERVE_X_ROLE, ScriptField::Lock),
            (RESERVE_Y_ROLE, ScriptField::Lock),
        ] {
            if cells::count_role(&ctx.script, role, source, field)? != expected {
                return Err(Error::Structure);
            }
        }
    }
    if let Some(before) = &pool.before {
        let action = cells::action::<Action>(before.index())?;
        if before.data().apply(&action)? != *pool.after.data() {
            return Err(Error::Transition);
        }
        let previous = syscall(load_input(before.index(), Source::Input))?.previous_output();
        if Unpack::<u32>::unpack(&previous.index()) != 0 {
            return Err(Error::Structure);
        }
        cells::require_same_origin(
            before.index(),
            pool.x_before.as_ref().ok_or(Error::Structure)?.index(),
            1,
        )?;
        cells::require_same_origin(
            before.index(),
            pool.y_before.as_ref().ok_or(Error::Structure)?.index(),
            2,
        )?;
    } else {
        if cells::creation_id(pool.after.index())? != args.id {
            return Err(Error::Genesis);
        }
        pool.after
            .data()
            .creation_settlement()
            .map_err(|_| Error::Genesis)?;
    }
    if let Some(before) = &pool.before {
        verify_reserves(
            before.data(),
            pool.x_before.as_ref().ok_or(Error::Structure)?,
            pool.y_before.as_ref().ok_or(Error::Structure)?,
        )?;
    }
    verify_reserves(pool.after.data(), &pool.x_after, &pool.y_after)?;
    InputAuthority::load_external(settlement(pool)?.actor, &ctx.script)?;
    Ok(())
}

fn verify_reserves(state: &State, x: &Cell<RawData>, y: &Cell<RawData>) -> Result<()> {
    for (asset, amount, cell) in [
        (state.asset_x, state.reserve_x, x),
        (state.asset_y, state.reserve_y, y),
    ] {
        let asset = crate::asset_type(asset);
        if !script::is_plain_xudt(&asset)
            || cell.type_script() != Some(&asset)
            || decode_amount(&cell.data().0, AmountEncoding::Exact16)? != amount
        {
            return Err(Error::Token);
        }
    }
    Ok(())
}
