use ckb_contract_core::{
    layout::{ReserveLayout, StateLayout},
    token::PlainXudtAsset,
    Error,
};
use ckb_contract_onchain::{
    context::Cells,
    patterns::{ReservePair, RoleEnvironment, StateCell},
};
use framework_counter_script::State;
use std::sync::atomic::{AtomicUsize, Ordering};

static STATE_READS: AtomicUsize = AtomicUsize::new(0);
static RESERVE_READS: AtomicUsize = AtomicUsize::new(0);

fn state_layout() -> StateLayout {
    STATE_READS.fetch_add(1, Ordering::SeqCst);
    StateLayout {
        role: 0,
        lock_role: 1,
        capacity: 400,
        output: 3,
    }
}
fn reserve_layout() -> ReserveLayout {
    RESERVE_READS.fetch_add(1, Ordering::SeqCst);
    ReserveLayout {
        role: 1,
        capacity: 200,
        output: 0,
    }
}

#[derive(Cells)]
struct Collision {
    #[state(layout = state_layout(), mode = "update")]
    state: StateCell<State>,
    #[reserve(layout = reserve_layout(), state = state,
        asset = PlainXudtAsset { owner: [1; 32], extended_args: false }, before = 0, after = 0)]
    reserve: ReservePair,
}

#[test]
fn shared_layout_values_are_evaluated_once_and_collisions_fail_before_syscalls() {
    let script = Default::default();
    assert!(matches!(
        Collision::load(&RoleEnvironment::new(&script, &[0; 32])),
        Err(Error::Encoding)
    ));
    assert_eq!(STATE_READS.load(Ordering::SeqCst), 1);
    assert_eq!(RESERVE_READS.load(Ordering::SeqCst), 1);
    let _read: fn(&Collision) -> (&StateCell<State>, &ReservePair) =
        |ctx| (&ctx.state, &ctx.reserve);
}
