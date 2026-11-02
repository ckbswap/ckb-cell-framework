use ckb_contract_core::{BinaryCodec, Error, StateTransition};
use ckb_contract_onchain::{cells, state::StateCells, Source};

#[derive(Debug, PartialEq)]
enum BusinessError {
    Framework(Error),
    InvalidValue,
    LimitReached,
}

impl From<Error> for BusinessError {
    fn from(error: Error) -> Self {
        Self::Framework(error)
    }
}

// Deliberately has no Clone implementation: validation does not require one.
#[derive(Debug, PartialEq)]
struct State(u8);

impl BinaryCodec for State {
    type Error = BusinessError;

    fn encode(&self) -> Vec<u8> {
        vec![self.0]
    }

    fn decode(data: &[u8]) -> Result<Self, Self::Error> {
        match data {
            [value] if *value <= 10 => Ok(Self(*value)),
            [_] => Err(BusinessError::InvalidValue),
            _ => Err(Error::Encoding.into()),
        }
    }
}

impl StateTransition for State {
    type Action = State;

    fn apply(&self, action: &State) -> Result<Self, Self::Error> {
        match self.0.checked_add(action.0) {
            Some(value) if value <= 10 => Ok(Self(value)),
            _ => Err(BusinessError::LimitReached),
        }
    }
}

#[test]
fn codec_and_transition_preserve_business_and_framework_errors() {
    assert_eq!(State::decode(&[11]), Err(BusinessError::InvalidValue));
    assert_eq!(
        State::decode(&[]),
        Err(BusinessError::Framework(Error::Encoding))
    );
    assert_eq!(State(8).apply(&State(3)), Err(BusinessError::LimitReached));
    assert_eq!(State(8).apply(&State(2)), Ok(State(10)));
}

#[test]
fn onchain_adapters_return_the_business_error_without_requiring_clone() {
    // Type-check the syscall adapters without executing syscalls on the host.
    let _: fn(usize, Source) -> Result<State, BusinessError> = cells::data::<State>;
    let _: fn(usize) -> Result<State, BusinessError> = cells::action::<State>;
    type UpdateResult = Result<(State, State, State), BusinessError>;
    let _: fn(&StateCells) -> UpdateResult = StateCells::verify_update::<State>;
}
