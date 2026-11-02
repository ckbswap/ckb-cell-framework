//! Exercise unrelated typed Cells and a dependency without a State pattern.
use ckb_contract_core::{Error, Result, StateTransition, CKB};
use ckb_contract_onchain::{
    ckb_types::packed::Script,
    context::{Cell, Cells, Context, RawData},
    Source,
};
use framework_counter_script::{Action, State};

#[derive(Cells)]
#[cells(args = Script)]
struct Batch {
    #[cell(source = Source::Input, index = 0, type_script = Some(args), capacity = 300 * CKB)]
    left: Cell<State>,
    #[cell(source = Source::Input, index = 1, type_script = Some(args), capacity = 300 * CKB)]
    right: Cell<State>,
    #[cell(source = Source::Output, index = 0, type_script = Some(args), lock = left.lock(), capacity = 300 * CKB)]
    left_after: Cell<State>,
    #[cell(source = Source::Output, index = 1, type_script = Some(args), lock = right.lock(), capacity = 300 * CKB)]
    right_after: Cell<State>,
    #[cell(source = Source::Output, index = 2, type_script = None)]
    optional: Option<Cell<State>>,
    #[cell(source = Source::CellDep, lock = args, type_script = None, capacity = 100 * CKB,
        constraint = config.data().0.as_slice() == [42])]
    config: Cell<RawData>,
}

pub fn verify(script: &Script) -> Result<()> {
    let ctx = Context::<Batch>::load(script)?;
    verify_batch(ctx)
}

fn verify_batch(ctx: Context<Batch>) -> Result<()> {
    if *ctx.cells.left_after.data() != ctx.cells.left.data().apply(&Action { increment: 1 })?
        || *ctx.cells.right_after.data()
            != ctx.cells.right.data().apply(&Action { increment: 2 })?
    {
        return Err(Error::Transition);
    }
    // This fixture's index policy intentionally requires two states exactly.
    if ckb_contract_onchain::cells::count(Source::GroupInput)? != 2
        || ckb_contract_onchain::cells::count(Source::GroupOutput)? != 2
    {
        return Err(Error::Structure);
    }
    let _ = (
        ctx.cells.left,
        ctx.cells.right,
        ctx.cells.left_after,
        ctx.cells.right_after,
        ctx.cells.config,
        ctx.cells.optional,
    );
    Ok(())
}
