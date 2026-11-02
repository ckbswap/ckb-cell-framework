use ckb_contract_core::{Error, Result};
use ckb_contract_onchain::context::Context;
use framework_counter_script::cells::{CounterArgs, Create, Update};
use framework_counter_script::{create, update, verify_update, Action, State};

#[test]
fn explicit_counter_validation_compares_the_actual_output_with_the_business_result() {
    let _: fn(Context<Create>, &CounterArgs) -> Result<()> = create;
    let _: fn(Context<Update>) -> Result<()> = update;
    let before = State {
        owner: [1; 32],
        nonce: 7,
        value: 10,
    };
    let action = Action { increment: 2 };
    let after = State {
        owner: before.owner,
        nonce: 8,
        value: 12,
    };
    assert_eq!(verify_update(&before, &action, &after), Ok(()));
    for forged in [
        State {
            owner: [2; 32],
            ..after.clone()
        },
        State {
            nonce: 7,
            ..after.clone()
        },
        State {
            value: 13,
            ..after.clone()
        },
    ] {
        assert_eq!(
            verify_update(&before, &action, &forged),
            Err(Error::Transition)
        );
    }
    assert_eq!(
        verify_update(&before, &Action { increment: 0 }, &before),
        Err(Error::Transition)
    );
    let overflow = State {
        value: u64::MAX,
        ..before
    };
    assert_eq!(
        verify_update(&overflow, &action, &after),
        Err(Error::Arithmetic)
    );
}
