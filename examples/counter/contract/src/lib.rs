//! Type-only Counter using framework Context containers and operation Cell collections.
#![no_std]
extern crate alloc;
pub mod cells;
pub mod state;
pub use state::*;

use self::cells::{CounterArgs, Create, Update};
use ckb_contract_core::{Error, Result, StateTransition, CKB};
use ckb_contract_onchain::{
    cells::{self as cell_checks, ScriptField},
    context::{Context, InputAuthority},
    state as state_cells, syscall, token, Source,
};
use ckb_std::high_level::load_script;

pub const STATE_CAPACITY: u64 = 300 * CKB;

pub fn create(ctx: Context<Create>, args: &CounterArgs) -> Result<()> {
    let after = &ctx.cells.after;
    state_cells::reject_type_as_input_lock(&ctx.script)?;
    token::reject_standard_mint(&[])?;
    if cell_checks::creation_id(after.index())? != args.id
        || *after.data() != State::new(after.data().owner)
    {
        return Err(Error::Genesis);
    }
    InputAuthority::load_external(after.data().owner, &ctx.script)?;
    Ok(())
}

pub fn update(ctx: Context<Update>) -> Result<()> {
    let before = &ctx.cells.before;
    let after = &ctx.cells.after;
    state_cells::reject_type_as_input_lock(&ctx.script)?;
    token::reject_standard_mint(&[])?;
    let action = cell_checks::action::<Action>(before.index())?;
    verify_update(before.data(), &action, after.data())?;
    InputAuthority::load_external(before.data().owner, &ctx.script)?;
    Ok(())
}

pub fn verify_update(before: &State, action: &Action, after: &State) -> Result<()> {
    if before.apply(action)? != *after {
        return Err(Error::Transition);
    }
    Ok(())
}

pub fn verify() -> Result<()> {
    let current = syscall(load_script())?;
    // This application chooses a raw 32-byte creation ID. Generic contexts do
    // not impose an args format, a witness convention or an output position.
    let raw_args = current.args().raw_data();
    let id: [u8; 32] = raw_args.as_ref().try_into().map_err(|_| Error::Encoding)?;
    let inputs = cell_checks::find(&current, Source::Input, ScriptField::Type)?;
    let args = CounterArgs {
        script: current,
        id,
    };
    if inputs.is_empty() {
        create(Context::<Create>::load(&args)?, &args)?;
    } else {
        update(Context::<Update>::load(&args)?)?;
    }
    Ok(())
}
