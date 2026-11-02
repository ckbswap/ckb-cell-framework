//! This application's Cell declarations and protocol choices. Roles, fixed
//! positions and reserve pairing are explicit business policy, not Context requirements.
use crate::{Action, State, LOCK_ROLE, RESERVE_CAPACITY, RESERVE_ROLE, STATE_CAPACITY, STATE_ROLE};
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

pub struct EscrowArgs {
    pub state_type: Script,
    pub state_lock: Script,
    pub reserve_lock: Script,
    pub id: Hash,
}

impl EscrowArgs {
    pub fn new(base: &Script, id: &Hash) -> Self {
        Self {
            state_type: script::with_role(base, STATE_ROLE, id, &[]),
            state_lock: script::with_role(base, LOCK_ROLE, id, &[]),
            reserve_lock: script::with_role(base, RESERVE_ROLE, id, &[]),
            id: *id,
        }
    }
}

#[derive(Cells)]
#[cells(args = EscrowArgs)]
pub struct EscrowCells {
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
        constraint = after.as_ref().is_none_or(|cell| {
            cell.index() == 0 && cell.lock() == &args.state_lock
        })
    )]
    pub after: Option<Cell<State>>,

    #[cell(
        source = Source::Input,
        lock = &args.reserve_lock,
        capacity = RESERVE_CAPACITY
    )]
    pub reserve_before: Option<Cell<RawData>>,

    #[cell(
        source = Source::Output,
        lock = &args.reserve_lock,
        capacity = RESERVE_CAPACITY,
        constraint = reserve_after.as_ref().is_none_or(|cell| cell.index() == 1)
    )]
    pub reserve_after: Option<Cell<RawData>>,
}

fn current<'a>(
    before: &'a Option<Cell<State>>,
    after: &'a Option<Cell<State>>,
) -> Result<&'a State> {
    before
        .as_ref()
        .or(after.as_ref())
        .map(Cell::data)
        .ok_or(Error::Structure)
}

fn actor(before: &Option<Cell<State>>, after: &Option<Cell<State>>) -> Result<Hash> {
    let state = current(before, after)?;
    if let (Some(before), Some(_)) = (before, after) {
        Ok(state
            .settlement(&cells::action::<Action>(before.index())?)?
            .actor)
    } else {
        Ok(state.buyer)
    }
}

fn check_initial(state: &State) -> Result<()> {
    if *state
        != State::new(state.buyer, state.seller, state.asset, state.amount)
            .map_err(|_| Error::Genesis)?
    {
        return Err(Error::Genesis);
    }
    Ok(())
}

/// Verify application-wide invariants, transitions, reserve pairing and
/// actor authorization. The handler calls this explicitly before settlement.
pub fn verify_protocol(ctx: &Context<EscrowCells>, args: &EscrowArgs) -> Result<()> {
    let before = &ctx.cells.before;
    let after = &ctx.cells.after;
    let reserve_before = &ctx.cells.reserve_before;
    let reserve_after = &ctx.cells.reserve_after;
    token::reject_standard_mint(&[])?;
    state_cells::reject_type_as_input_lock(&args.state_type)?;
    for (source, state, reserve) in [
        (Source::Input, before, reserve_before),
        (Source::Output, after, reserve_after),
    ] {
        if state.is_some() != reserve.is_some() {
            return Err(Error::Structure);
        }
        for (role, field) in [
            (STATE_ROLE, ScriptField::Type),
            (LOCK_ROLE, ScriptField::Lock),
            (RESERVE_ROLE, ScriptField::Lock),
        ] {
            if cells::count_role(&ctx.script, role, source, field)? != usize::from(state.is_some())
            {
                return Err(Error::Structure);
            }
        }
    }
    match (before, after) {
        (None, Some(after)) => {
            if cells::creation_id(after.index())? != args.id {
                return Err(Error::Genesis);
            }
            check_initial(after.data())?;
        }
        (Some(before), Some(after)) => {
            let action = cells::action::<Action>(before.index())?;
            if before.data().apply(&action)? != *after.data() {
                return Err(Error::Transition);
            }
        }
        (Some(before), None) => {
            if cells::action::<Action>(before.index())? != Action::Close {
                return Err(Error::Transition);
            }
            before.data().require_close()?;
        }
        (None, None) => return Err(Error::Structure),
    }
    if let Some(before) = before {
        let previous = syscall(load_input(before.index(), Source::Input))?.previous_output();
        if Unpack::<u32>::unpack(&previous.index()) != 0 {
            return Err(Error::Structure);
        }
        cells::require_same_origin(
            before.index(),
            reserve_before.as_ref().ok_or(Error::Structure)?.index(),
            1,
        )?;
    }
    for (state, reserve) in [(before, reserve_before), (after, reserve_after)] {
        if let (Some(state), Some(reserve)) = (state, reserve) {
            let state = state.data();
            let asset = crate::asset_type(state);
            if !script::is_plain_xudt(&asset)
                || reserve.type_script() != Some(&asset)
                || decode_amount(&reserve.data().0, AmountEncoding::Exact16)?
                    != state.reserve_amount()
            {
                return Err(Error::Token);
            }
        }
    }
    InputAuthority::load_external(actor(before, after)?, &ctx.script)?;
    Ok(())
}
