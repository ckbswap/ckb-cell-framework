#![no_std]
extern crate alloc;
pub mod cells;
pub mod state;
pub use cells::VestingCells;
pub use state::*;

use ckb_contract_core::{
    args::RoleArgs,
    check_flow,
    layout::{ReserveLayout, StateLayout, TransactionLayout},
    token::AmountEncoding,
    Error, Result, CKB,
};
use ckb_contract_onchain::{
    cells as cell_checks, context::Context, script, state as state_cells, syscall, token, Source,
};
use ckb_std::ckb_types::{packed::Script, prelude::*};
use ckb_std::high_level::load_script;

pub const LAYOUT: TransactionLayout<'static> = TransactionLayout {
    state: StateLayout {
        role: 0,
        lock_role: 1,
        capacity: 400 * CKB,
        output: 0,
    },
    reserves: &[ReserveLayout {
        role: 2,
        capacity: 200 * CKB,
        output: 1,
    }],
};

// Named roles and capacities used by verification and test fixtures.
pub const STATE_ROLE: u8 = LAYOUT.state.role;
pub const LOCK_ROLE: u8 = LAYOUT.state.lock_role;
pub const RESERVE_ROLE: u8 = LAYOUT.reserves[0].role;
pub const STATE_CAPACITY: u64 = LAYOUT.state.capacity;
pub const RESERVE_CAPACITY: u64 = LAYOUT.reserves[0].capacity;

pub fn verify() -> Result<()> {
    let current = syscall(load_script())?;
    let raw_args = current.args().raw_data();
    let args = RoleArgs::decode(&raw_args)?;
    args.require_payload_len(0)?;
    let state_type = script::with_role(&current, STATE_ROLE, &args.id, &[]);
    match args.role {
        STATE_ROLE => {
            let args = self::cells::VestingArgs::new(&current, &args.id);
            verify_grant(Context::<VestingCells>::load(&args)?, &args)
        }
        LOCK_ROLE => state_cells::verify_state_lock(&state_type),
        RESERVE_ROLE => state_cells::require_state_input(&state_type).map(|_| ()),
        _ => Err(Error::Encoding),
    }
}

pub fn verify_grant(ctx: Context<VestingCells>, args: &self::cells::VestingArgs) -> Result<()> {
    self::cells::verify_protocol(&ctx, args)?;
    verify_settlement(&ctx)
}

fn verify_settlement(ctx: &Context<VestingCells>) -> Result<()> {
    match (&ctx.cells.before, &ctx.cells.after) {
        (None, Some(after)) => verify_asset_flow(after.data(), after.data().total, 0),
        (Some(before), Some(_)) => {
            let Action::Claim { amount, .. } = cell_checks::action::<Action>(before.index())?
            else {
                return Err(Error::Transition);
            };
            verify_asset_flow(before.data(), 0, amount)
        }
        (Some(before), None) => {
            verify_asset_flow(before.data(), 0, 0)?;
            cell_checks::require_payment(
                &before.data().funder,
                STATE_CAPACITY + RESERVE_CAPACITY,
                cell_checks::transaction_fee()?,
            )
        }
        _ => Err(Error::Structure),
    }
}

fn asset_type(grant: &State) -> Script {
    script::plain_xudt(&grant.asset.owner)
        .as_builder()
        .args(grant.asset.args().pack())
        .build()
}

/// Funder supplies the principal; the beneficiary receives exact net claims.
/// When both roles use the same wallet, check that wallet only once.
fn verify_asset_flow(grant: &State, spent: u128, received: u128) -> Result<()> {
    let hash = script::script_hash(&asset_type(grant));
    token::require_supply_change(&hash, 0, 0)?;
    for (index, party) in [&grant.funder, &grant.beneficiary].into_iter().enumerate() {
        if index == 1 && grant.funder == grant.beneficiary {
            continue;
        }
        check_flow(
            token::total(&hash, Source::Input, Some(party), AmountEncoding::Exact16)?,
            token::total(&hash, Source::Output, Some(party), AmountEncoding::Exact16)?,
            if *party == grant.funder { spent } else { 0 },
            if *party == grant.beneficiary {
                received
            } else {
                0
            },
        )?;
    }
    Ok(())
}

/// Export the actual derived context and codec schemas. Operation bindings are
/// explicit: arbitrary protocol/authorization functions are not inferred.
#[cfg(feature = "idl")]
pub fn interface() -> ckb_contract_idl::Interface<'static> {
    use crate::{Action, State};
    use ckb_contract_core::wire::CodecSchema;
    use ckb_contract_idl::{ContextBindings, Instruction, Interface, Phase};
    use ckb_contract_onchain::context::Cells;

    Interface {
        name: "vesting",
        version: env!("CARGO_PKG_VERSION"),
        state: State::SCHEMA,
        action: Action::SCHEMA,
        layout: LAYOUT,
        context: VestingCells::schema(),
        bindings: Some(ContextBindings {
            state_output: "after",
            reserve_outputs: &["reserve_after"],
        }),
        instructions: &[
            Instruction {
                name: "create",
                phase: Phase::Create,
                action: None,
                authority: "funder",
                timestamp: None,
            },
            Instruction {
                name: "claim",
                phase: Phase::Update,
                action: Some("Claim"),
                authority: "beneficiary",
                timestamp: Some("timestamp"),
            },
            Instruction {
                name: "close",
                phase: Phase::Close,
                action: Some("Close"),
                authority: "funder",
                timestamp: None,
            },
        ],
    }
}
