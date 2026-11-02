//! Example business rules. No CKB syscalls.
use ckb_contract_core::{Error, Hash, Result, StateTransition};
use ckb_contract_derive::CellCodec;

#[derive(Clone, Debug, PartialEq, Eq, CellCodec)]
#[codec(prefix = b"CNT01")]
pub struct State {
    pub owner: Hash,
    pub nonce: u64,
    pub value: u64,
}

impl State {
    pub fn new(owner: Hash) -> Self {
        Self {
            owner,
            nonce: 0,
            value: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, CellCodec)]
pub struct Action {
    pub increment: u64,
}

impl StateTransition for State {
    type Action = Action;

    fn apply(&self, action: &Action) -> Result<Self> {
        if action.increment == 0 {
            return Err(Error::Transition);
        }
        Ok(Self {
            owner: self.owner,
            nonce: self.nonce.checked_add(1).ok_or(Error::Arithmetic)?,
            value: self
                .value
                .checked_add(action.increment)
                .ok_or(Error::Arithmetic)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckb_contract_core::BinaryCodec;

    #[test]
    fn counter_transition_preserves_owner_and_rejects_overflow() {
        let before = State::new([1; 32]);
        let after = before.apply(&Action { increment: 7 }).unwrap();
        assert_eq!(
            after,
            State {
                owner: before.owner,
                nonce: 1,
                value: 7
            }
        );
        assert_eq!(
            before.apply(&Action { increment: 0 }),
            Err(Error::Transition)
        );
        let mut max = before.clone();
        max.value = u64::MAX;
        assert_eq!(max.apply(&Action { increment: 1 }), Err(Error::Arithmetic));
        max = before;
        max.nonce = u64::MAX;
        assert_eq!(max.apply(&Action { increment: 1 }), Err(Error::Arithmetic));
    }

    #[test]
    fn codecs_reject_alternate_magic_and_trailing_data() {
        let state = State::new([1; 32]);
        assert_eq!(state.encode().len(), 53);
        assert_eq!(State::decode(&state.encode()), Ok(state.clone()));
        let mut bytes = state.encode();
        bytes[0] = 0;
        assert_eq!(State::decode(&bytes), Err(Error::Encoding));
        assert_eq!(Action::decode(&[0; 9]), Err(Error::Encoding));
    }

    #[test]
    fn derived_codecs_preserve_the_original_counter_wire_format() {
        let state = State {
            owner: [0x11; 32],
            nonce: 0x0102_0304_0506_0708,
            value: 0x1112_1314_1516_1718,
        };
        let bytes = [
            b"CNT01".as_slice(),
            &[0x11; 32],
            &[8, 7, 6, 5, 4, 3, 2, 1],
            &[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11],
        ]
        .concat();
        assert_eq!(state.encode(), bytes);
        assert_eq!(State::decode(&bytes), Ok(state));
        let action = Action {
            increment: 0x0102_0304_0506_0708,
        };
        assert_eq!(action.encode(), [8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(Action::decode(&[8, 7, 6, 5, 4, 3, 2, 1]), Ok(action));
        for len in 0..bytes.len() {
            assert_eq!(State::decode(&bytes[..len]), Err(Error::Encoding));
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert_eq!(State::decode(&trailing), Err(Error::Encoding));
    }
}
