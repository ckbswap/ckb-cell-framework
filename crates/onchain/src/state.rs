use crate::{
    cells::{self, ScriptField},
    syscall, Source,
};
use ckb_contract_core::{Error, Result, StateTransition};
use ckb_std::{ckb_types::packed::Script, high_level::*};

/// A persistent single-State lifecycle: 0->1 creation or 1->1 update; no destruction.
/// Counts exact full Scripts; multi-entity transaction policy belongs to the caller.
pub struct StateCells {
    pub input: Option<usize>,
    pub output: usize,
}

impl StateCells {
    pub fn load(
        state_type: &Script,
        state_lock: &Script,
        output_index: Option<usize>,
    ) -> Result<Self> {
        reject_type_as_input_lock(state_type)?;
        let inputs = cells::find(state_type, Source::Input, ScriptField::Type)?;
        let output = cells::exactly_one(state_type, Source::Output, ScriptField::Type)?;
        if inputs.len() > 1 || output_index.is_some_and(|expected| output != expected) {
            return Err(Error::Structure);
        }
        if syscall(load_cell_lock(output, Source::Output))? != *state_lock {
            return Err(Error::Structure);
        }
        if let Some(&input) = inputs.first() {
            if syscall(load_cell_lock(input, Source::Input))? != *state_lock {
                return Err(Error::Structure);
            }
        }
        Ok(Self {
            input: inputs.first().copied(),
            output,
        })
    }

    pub fn require_capacity(&self, capacity: u64) -> Result<()> {
        if syscall(load_cell_capacity(self.output, Source::Output))? != capacity {
            return Err(Error::Capacity);
        }
        if let Some(input) = self.input {
            if syscall(load_cell_capacity(input, Source::Input))? != capacity {
                return Err(Error::Capacity);
            }
        }
        Ok(())
    }

    /// Decode the action from the actual State input index, not hard-coded input 0.
    pub fn verify_update<S: StateTransition>(
        &self,
    ) -> ::core::result::Result<(S, S::Action, S), S::Error> {
        let input = self.input.ok_or(Error::Structure)?;
        let before = cells::data::<S>(input, Source::Input)?;
        let action = cells::action::<S::Action>(input)?;
        let after = cells::data::<S>(self.output, Source::Output)?;
        if before.apply(&action)? != after {
            return Err(Error::Transition.into());
        }
        Ok((before, action, after))
    }
}

/// Explicit 1->0 lifecycle. Business close permission, reserves and refunds
/// remain the caller's responsibility; StateCells still forbids destruction.
pub struct ClosingStateCell {
    pub input: usize,
}

impl ClosingStateCell {
    pub fn load(state_type: &Script, state_lock: &Script) -> Result<Self> {
        reject_type_as_input_lock(state_type)?;
        let input = cells::exactly_one(state_type, Source::Input, ScriptField::Type)?;
        if !cells::find(state_type, Source::Output, ScriptField::Type)?.is_empty()
            || syscall(load_cell_lock(input, Source::Input))? != *state_lock
        {
            return Err(Error::Structure);
        }
        Ok(Self { input })
    }

    pub fn require_capacity(&self, capacity: u64) -> Result<()> {
        if syscall(load_cell_capacity(self.input, Source::Input))? != capacity {
            return Err(Error::Capacity);
        }
        Ok(())
    }
}

pub fn reject_type_as_input_lock(state_type: &Script) -> Result<()> {
    if !cells::find(state_type, Source::Input, ScriptField::Lock)?.is_empty() {
        return Err(Error::Authorization);
    }
    Ok(())
}

/// Shared public State Lock: the corresponding Type performs all business checks.
pub fn verify_state_lock(state_type: &Script) -> Result<()> {
    if cells::count(Source::GroupInput)? != 1
        || syscall(load_cell_type(0, Source::GroupInput))?.as_ref() != Some(state_type)
    {
        return Err(Error::Structure);
    }
    Ok(())
}

/// Public reserve/companion lock. Pair with exact structure, balances and origin checks.
pub fn require_state_input(state_type: &Script) -> Result<usize> {
    cells::exactly_one(state_type, Source::Input, ScriptField::Type)
}
