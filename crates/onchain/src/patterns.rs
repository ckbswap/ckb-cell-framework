//! Optional single-State protocol: role scripts, lifecycle transitions and paired reserves.
//! Generic Cell contexts live in `crate::context` and do not require these conventions.
use crate::{
    cells::{self, ScriptField},
    script, state, syscall, token, Source,
};
use ckb_contract_core::{
    layout::{validate_layout, StateLayout},
    token::PlainXudtAsset,
    BinaryCodec, Error, Hash, Result, StateTransition,
};
use ckb_std::{
    ckb_types::{packed::Script, prelude::*},
    high_level::load_input,
};

pub struct RoleEnvironment<'a> {
    pub base: &'a Script,
    pub id: &'a Hash,
}

impl<'a> RoleEnvironment<'a> {
    pub fn new(base: &'a Script, id: &'a Hash) -> Self {
        Self { base, id }
    }

    pub fn role(&self, role: u8) -> Script {
        script::with_role(self.base, role, self.id, &[])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Create,
    Update,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Create,
    Update,
    Close,
    Persist,
    Any,
}

impl Lifecycle {
    fn accepts(self, phase: Phase) -> bool {
        matches!(
            (self, phase),
            (Self::Any, _)
                | (Self::Persist, Phase::Create | Phase::Update)
                | (Self::Create, Phase::Create)
                | (Self::Update, Phase::Update)
                | (Self::Close, Phase::Close)
        )
    }
}

type Check<S> = fn(&S) -> ::core::result::Result<(), <S as BinaryCodec>::Error>;

pub struct StatePolicy<S: StateTransition> {
    pub role: u8,
    pub lock_role: u8,
    pub capacity: u64,
    pub output: usize,
    pub lifecycle: Lifecycle,
    pub initial: Option<Check<S>>,
    pub close: Option<Check<S>>,
    pub close_action: Option<S::Action>,
    pub single_state: bool,
}

pub struct StateCell<S: StateTransition> {
    phase: Phase,
    before: Option<S>,
    after: Option<S>,
    action: Option<S::Action>,
    input: Option<usize>,
    output: Option<usize>,
}

impl<S: StateTransition> StateCell<S>
where
    S::Action: PartialEq,
{
    /// Convenience path used by derived contexts. Executes business callbacks
    /// and State::apply in addition to loading and checking Cell structure.
    pub fn load_and_verify(
        env: &RoleEnvironment<'_>,
        policy: StatePolicy<S>,
    ) -> ::core::result::Result<Self, S::Error> {
        let cells = Self::load_cells(
            env,
            StateLayout {
                role: policy.role,
                lock_role: policy.lock_role,
                capacity: policy.capacity,
                output: policy.output,
            },
            policy.lifecycle,
            policy.single_state,
        )?;
        match cells.phase {
            Phase::Create => {
                policy.initial.ok_or(Error::Genesis)?(
                    cells.after.as_ref().ok_or(Error::Structure)?,
                )?;
            }
            Phase::Update => {
                let expected = cells
                    .before
                    .as_ref()
                    .ok_or(Error::Structure)?
                    .apply(cells.action.as_ref().ok_or(Error::Structure)?)?;
                if cells.after.as_ref() != Some(&expected) {
                    return Err(Error::Transition.into());
                }
            }
            Phase::Close => {
                if cells.action != policy.close_action || policy.close_action.is_none() {
                    return Err(Error::Transition.into());
                }
                policy.close.ok_or(Error::Transition)?(
                    cells.before.as_ref().ok_or(Error::Structure)?,
                )?;
            }
        }
        Ok(cells)
    }

    /// Locate and decode Cells; check full Scripts, capacity, positions,
    /// lifecycle and creation identity. Does not call State::apply, authorize
    /// an actor, validate initial/terminal business state or check settlement.
    /// The caller performs those checks explicitly after loading.
    pub fn load_cells(
        env: &RoleEnvironment<'_>,
        policy: StateLayout,
        lifecycle: Lifecycle,
        single_state: bool,
    ) -> ::core::result::Result<Self, S::Error> {
        validate_layout(&[policy.role, policy.lock_role], &[policy.output])?;
        let state_type = env.role(policy.role);
        let state_lock = env.role(policy.lock_role);
        state::reject_type_as_input_lock(&state_type)?;
        let has_output = !cells::find(&state_type, Source::Output, ScriptField::Type)?.is_empty();
        let (input, output) = if has_output {
            let pair = state::StateCells::load(&state_type, &state_lock, Some(policy.output))?;
            pair.require_capacity(policy.capacity)?;
            (pair.input, Some(pair.output))
        } else {
            let closing = state::ClosingStateCell::load(&state_type, &state_lock)?;
            closing.require_capacity(policy.capacity)?;
            (Some(closing.input), None)
        };
        let phase = match (input, output) {
            (None, Some(_)) => Phase::Create,
            (Some(_), Some(_)) => Phase::Update,
            (Some(_), None) => Phase::Close,
            _ => return Err(Error::Structure.into()),
        };
        if !lifecycle.accepts(phase) {
            return Err(Error::Structure.into());
        }
        if single_state {
            for (source, expected) in [
                (Source::Input, usize::from(input.is_some())),
                (Source::Output, usize::from(output.is_some())),
            ] {
                for (role, field) in [
                    (policy.role, ScriptField::Type),
                    (policy.lock_role, ScriptField::Lock),
                ] {
                    if cells::count_role(env.base, role, source, field)? != expected {
                        return Err(Error::Structure.into());
                    }
                }
            }
        }
        if let Some(index) = input {
            let point = syscall(load_input(index, Source::Input))?.previous_output();
            if u64::from(Unpack::<u32>::unpack(&point.index())) != policy.output as u64 {
                return Err(Error::Structure.into());
            }
        }
        let before = input
            .map(|i| cells::data::<S>(i, Source::Input))
            .transpose()?;
        let after = output
            .map(|i| cells::data::<S>(i, Source::Output))
            .transpose()?;
        let action = input.map(cells::action::<S::Action>).transpose()?;
        if phase == Phase::Create {
            let index = output.ok_or(Error::Structure)?;
            if cells::creation_id(index)? != *env.id {
                return Err(Error::Genesis.into());
            }
        }
        Ok(Self {
            phase,
            before,
            after,
            action,
            input,
            output,
        })
    }
}

impl<S: StateTransition> StateCell<S> {
    /// Bind an update's Action timestamp (Unix seconds) to this State input's
    /// canonical absolute timestamp since. Creation and close do not evaluate
    /// the callback. Consensus, separately, must enforce since maturity.
    pub fn require_update_timestamp(
        &self,
        timestamp: impl FnOnce(&S::Action) -> ::core::result::Result<u64, S::Error>,
    ) -> ::core::result::Result<(), S::Error> {
        if self.phase == Phase::Update {
            let input = self.input.ok_or(Error::Structure)?;
            let action = self.action.as_ref().ok_or(Error::Structure)?;
            cells::require_timestamp_since(input, timestamp(action)?)?;
        }
        Ok(())
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn before(&self) -> Option<&S> {
        self.before.as_ref()
    }

    pub fn after(&self) -> Option<&S> {
        self.after.as_ref()
    }

    pub fn state(&self) -> &S {
        // load constructs a context only for one of the three valid lifecycles.
        self.before
            .as_ref()
            .or(self.after.as_ref())
            .expect("verified State missing")
    }

    pub fn action(&self) -> Option<&S::Action> {
        self.action.as_ref()
    }

    pub fn input(&self) -> Option<usize> {
        self.input
    }

    pub fn output(&self) -> Option<usize> {
        self.output
    }
}

pub struct ReservePolicy {
    pub role: u8,
    pub asset: PlainXudtAsset,
    pub capacity: u64,
    pub output: usize,
    pub before: u128,
    pub after: u128,
}

pub struct ReservePair {
    input: Option<usize>,
    output: Option<usize>,
}

impl ReservePair {
    pub fn load<S: StateTransition>(
        env: &RoleEnvironment<'_>,
        state: &StateCell<S>,
        policy: ReservePolicy,
    ) -> Result<Self> {
        let output_index: u32 = policy.output.try_into().map_err(|_| Error::Structure)?;
        let lock = env.role(policy.role);
        let asset = script::plain_xudt(&policy.asset.owner)
            .as_builder()
            .args(policy.asset.args().pack())
            .build();
        for (source, expected) in [
            (Source::Input, usize::from(state.input.is_some())),
            (Source::Output, usize::from(state.output.is_some())),
        ] {
            if cells::count_role(env.base, policy.role, source, ScriptField::Lock)? != expected {
                return Err(Error::Structure);
            }
        }
        let verify = |source, amount| {
            token::ReserveSpec {
                lock: &lock,
                token_type: &asset,
                amount,
                capacity: policy.capacity,
            }
            .verify(source)
        };
        let input = if let Some(state_input) = state.input {
            let index = verify(Source::Input, policy.before)?;
            cells::require_same_origin(state_input, index, output_index)?;
            Some(index)
        } else {
            None
        };
        let output = if state.output.is_some() {
            let index = verify(Source::Output, policy.after)?;
            if index != policy.output {
                return Err(Error::Structure);
            }
            Some(index)
        } else {
            None
        };
        Ok(Self { input, output })
    }

    pub fn input(&self) -> Option<usize> {
        self.input
    }

    pub fn output(&self) -> Option<usize> {
        self.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_timestamp_does_not_require_an_action_for_creation_or_close() {
        for phase in [Phase::Create, Phase::Close] {
            let state = StateCell::<framework_counter_script::State> {
                phase,
                before: None,
                after: None,
                action: None,
                input: None,
                output: None,
            };
            assert_eq!(
                state.require_update_timestamp(|_| panic!("non-update evaluated timestamp")),
                Ok(())
            );
        }
    }

    #[test]
    fn update_timestamp_preserves_projection_errors_and_requires_state_input() {
        let mut state = StateCell::<framework_counter_script::State> {
            phase: Phase::Update,
            before: None,
            after: None,
            action: Some(framework_counter_script::Action { increment: 1 }),
            input: Some(3),
            output: Some(0),
        };
        assert_eq!(
            state.require_update_timestamp(|_| Err(Error::Authorization)),
            Err(Error::Authorization)
        );
        state.input = None;
        assert_eq!(
            state.require_update_timestamp(|_| panic!("missing State input accepted")),
            Err(Error::Structure)
        );
        state.input = Some(3);
        state.action = None;
        assert_eq!(
            state.require_update_timestamp(|_| panic!("missing Action accepted")),
            Err(Error::Structure)
        );
    }

    #[test]
    fn role_and_output_collisions_are_rejected_before_loading_cells() {
        assert_eq!(validate_layout(&[0, 1, 2], &[0, 1]), Ok(()));
        assert_eq!(validate_layout(&[0, 1, 1], &[0, 1]), Err(Error::Encoding));
        assert_eq!(validate_layout(&[0, 1, 2], &[0, 0]), Err(Error::Structure));
        if usize::BITS > 32 {
            assert_eq!(
                validate_layout(&[0, 1], &[(u32::MAX as usize) + 1]),
                Err(Error::Structure)
            );
        }
    }

    #[test]
    fn lifecycle_modes_allow_only_declared_input_output_transitions() {
        for (mode, allowed) in [
            (Lifecycle::Create, [true, false, false]),
            (Lifecycle::Update, [false, true, false]),
            (Lifecycle::Close, [false, false, true]),
            (Lifecycle::Persist, [true, true, false]),
            (Lifecycle::Any, [true, true, true]),
        ] {
            for (phase, expected) in [Phase::Create, Phase::Update, Phase::Close]
                .into_iter()
                .zip(allowed)
            {
                assert_eq!(mode.accepts(phase), expected);
            }
        }
    }
}
