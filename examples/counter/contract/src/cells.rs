//! Cell collections for the Type-only Counter. Business verification stays in handlers.
use crate::{State, STATE_CAPACITY};
use ckb_contract_core::Hash;
use ckb_contract_onchain::{
    context::{Cell, Cells},
    script, Source,
};
use ckb_std::ckb_types::packed::Script;

pub struct CounterArgs {
    pub script: Script,
    pub id: Hash,
}

#[derive(Cells)]
#[cells(args = CounterArgs)]
pub struct Create {
    #[cell(
        source = Source::Output,
        type_script = Some(&args.script),
        capacity = STATE_CAPACITY,
        constraint = script::script_hash(after.lock()) == after.data().owner
    )]
    pub after: Cell<State>,
}

#[derive(Cells)]
#[cells(args = CounterArgs)]
pub struct Update {
    #[cell(
        source = Source::Input,
        type_script = Some(&args.script),
        capacity = STATE_CAPACITY,
        constraint = script::script_hash(before.lock()) == before.data().owner
    )]
    pub before: Cell<State>,

    #[cell(
        source = Source::Output,
        type_script = Some(&args.script),
        lock = before.lock(),
        capacity = STATE_CAPACITY
    )]
    pub after: Cell<State>,
}
