use ckb_contract_core::{BinaryCodec, Error, StateTransition};
use ckb_contract_onchain::{
    context::{Cells, InputAuthority},
    patterns::{RoleEnvironment, StateCell},
};

#[derive(Debug, PartialEq)]
enum AppError {
    Framework(Error),
}

impl From<Error> for AppError {
    fn from(error: Error) -> Self {
        Self::Framework(error)
    }
}

// No Clone is required for State or Action.
#[derive(PartialEq)]
struct State(u8);

impl BinaryCodec for State {
    type Error = AppError;

    fn encode(&self) -> Vec<u8> {
        vec![self.0]
    }

    fn decode(data: &[u8]) -> Result<Self, AppError> {
        match data {
            [value] => Ok(Self(*value)),
            _ => Err(Error::Encoding.into()),
        }
    }
}

impl StateTransition for State {
    type Action = State;

    fn apply(&self, action: &State) -> Result<Self, AppError> {
        Ok(State(
            self.0.checked_add(action.0).ok_or(Error::Arithmetic)?,
        ))
    }
}

#[derive(Cells)]
struct Update {
    #[state(
        role = 0,
        lock_role = 1,
        capacity = 30_000_000_000,
        output = 0,
        mode = "update",
        update_timestamp = timestamp
    )]
    env: StateCell<State>,
    #[authority(lock_hash = [1; 32])]
    owner: InputAuthority,
}

fn timestamp(action: &State) -> Result<u64, AppError> {
    Ok(u64::from(action.0))
}

#[test]
fn context_preserves_business_error_and_does_not_require_clone() {
    let _: fn(&RoleEnvironment<'_>) -> Result<Update, AppError> = Update::load;
    assert_eq!(Update::schema().fields[0].name, "env");
    // Read the private test field in a type-checked function, without syscalls.
    let _read: fn(&Update) -> (&State, &InputAuthority) = |ctx| (ctx.env.state(), &ctx.owner);
}
