#![no_std]
#![no_main]
// This fixture exercises layout/constraint agreement; it does not define an
// asset settlement model and is not an application template.
use ckb_contract_core::{args::RoleArgs, Error, Result};
use ckb_contract_onchain::{
    context::{Cells, Context, InputAuthority},
    patterns::{ReservePair, RoleEnvironment, StateCell},
    state, syscall,
};
use ckb_std::high_level::load_script;
use framework_counter_script::State;

mod config;
mod generic;
use config::{ASSETS, LAYOUT};

#[derive(Cells)]
#[cells(no_mint, single_state)]
pub struct LayoutCells {
    #[state(layout = LAYOUT.state, mode = "persist", initial = initial)]
    pub state: StateCell<State>,
    #[authority(lock_hash = state.state().owner)]
    pub owner: InputAuthority,
    #[reserve(layout = LAYOUT.reserves[0], state = state, asset = ASSETS[0], before = 100, after = 100)]
    pub first: ReservePair,
    #[reserve(layout = LAYOUT.reserves[1], state = state, asset = ASSETS[1], before = 200, after = 200)]
    pub second: ReservePair,
}

fn initial(state: &State) -> Result<()> {
    if *state != State::new(state.owner) {
        return Err(Error::Genesis);
    }
    Ok(())
}

ckb_std::entry!(entry);
ckb_std::default_alloc!();

fn entry() -> i8 {
    verify().map_or_else(|error| error as i8, |_| 0)
}

pub fn verify() -> Result<()> {
    let current = syscall(load_script())?;
    let raw_args = current.args().raw_data();
    if raw_args.is_empty() {
        return generic::verify(&current);
    }
    let args = RoleArgs::decode(&raw_args)?;
    args.require_payload_len(0)?;
    let env = RoleEnvironment::new(&current, &args.id);
    let state_type = env.role(LAYOUT.state.role);
    if args.role == LAYOUT.state.role {
        Context::<LayoutCells>::load(&env).map(|_| ())
    } else if args.role == LAYOUT.state.lock_role {
        state::verify_state_lock(&state_type)
    } else if LAYOUT
        .reserves
        .iter()
        .any(|reserve| reserve.role == args.role)
    {
        state::require_state_input(&state_type).map(|_| ())
    } else {
        Err(Error::Encoding)
    }
}
