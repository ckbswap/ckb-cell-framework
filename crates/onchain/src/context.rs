//! Typed Cells and declarative transaction checks, independent of script args,
//! state machines, asset custody and output layouts.
use crate::{cells, syscall, Source};
use alloc::vec::Vec;
use ckb_contract_core::{schema::ContextSchema, BinaryCodec, Error, Hash, Result};
use ckb_std::{ckb_types::packed::Script, error::SysError, high_level::*};

pub use ckb_contract_derive::Cells;

pub trait Cells: Sized {
    type Error: From<Error>;
    /// Caller-supplied scripts, indices or other operation parameters. The
    /// optional State pattern uses a RoleEnvironment instead.
    type Args<'a>;

    /// Load declared fields and execute only their declared constraints.
    /// Application protocol, authorization and settlement checks belong in
    /// explicit methods called after loading. Optional pattern fields may
    /// themselves perform the validation that their wrappers document.
    fn load(args: &Self::Args<'_>) -> core::result::Result<Self, Self::Error>;

    fn schema() -> ContextSchema;
}

/// Framework-owned execution container. `C` is the application's declared Cell
/// collection. Loading checks its declarations; handlers explicitly verify
/// protocol rules and settlement using `ctx.cells` and the current `ctx.script`.
pub struct Context<C> {
    pub cells: C,
    pub script: Script,
}

impl<C: Cells> Context<C> {
    /// Load the executing Script and the declared Cell collection. This does
    /// not call application handlers or infer undeclared business checks.
    pub fn load(args: &C::Args<'_>) -> core::result::Result<Self, C::Error> {
        let script = syscall(load_script())?;
        let cells = C::load(args)?;
        Ok(Self { cells, script })
    }
}

#[derive(Clone, Copy, Default)]
pub enum TypeConstraint<'a> {
    #[default]
    Any,
    Absent,
    Exact(&'a Script),
}

impl<'a> From<Option<&'a Script>> for TypeConstraint<'a> {
    fn from(script: Option<&'a Script>) -> Self {
        script.map_or(Self::Absent, Self::Exact)
    }
}

/// With an index, check that exact Cell. Without an index, require exactly one
/// Cell matching the declared full scripts. Capacity and data are checked only
/// after selection, so they cannot hide a second matching Cell.
pub struct CellPolicy<'a> {
    pub source: Source,
    pub index: Option<usize>,
    pub lock: Option<&'a Script>,
    pub type_script: TypeConstraint<'a>,
    pub capacity: Option<u64>,
}

pub struct Cell<T: BinaryCodec> {
    source: Source,
    index: usize,
    capacity: u64,
    lock: Script,
    type_script: Option<Script>,
    data: T,
}

impl<T: BinaryCodec> Cell<T> {
    pub fn load(policy: CellPolicy<'_>) -> core::result::Result<Self, T::Error> {
        Self::load_optional(policy)?.ok_or_else(|| Error::Structure.into())
    }

    /// Absence is allowed; duplicates, mismatched indexed Cells, invalid data
    /// and syscall errors still fail. Optionality does not relax validation.
    pub fn load_optional(policy: CellPolicy<'_>) -> core::result::Result<Option<Self>, T::Error> {
        if policy.index.is_none()
            && policy.lock.is_none()
            && matches!(policy.type_script, TypeConstraint::Any)
        {
            return Err(Error::Structure.into());
        }
        let matches = |index| -> Result<bool> {
            if let Some(lock) = policy.lock {
                if syscall(load_cell_lock(index, policy.source))? != *lock {
                    return Ok(false);
                }
            }
            Ok(match policy.type_script {
                TypeConstraint::Any => true,
                TypeConstraint::Absent => syscall(load_cell_type(index, policy.source))?.is_none(),
                TypeConstraint::Exact(expected) => {
                    syscall(load_cell_type(index, policy.source))?.as_ref() == Some(expected)
                }
            })
        };
        let index = if let Some(index) = policy.index {
            match load_cell_capacity(index, policy.source) {
                Err(SysError::IndexOutOfBound) => return Ok(None),
                Err(_) => return Err(Error::Structure.into()),
                Ok(_) => {}
            }
            if !matches(index)? {
                return Err(Error::Structure.into());
            }
            index
        } else {
            let mut selected = None;
            for index in 0..cells::count(policy.source)? {
                if matches(index)? && selected.replace(index).is_some() {
                    return Err(Error::Structure.into());
                }
            }
            let Some(index) = selected else {
                return Ok(None);
            };
            index
        };
        let capacity = syscall(load_cell_capacity(index, policy.source))?;
        if policy.capacity.is_some_and(|expected| capacity != expected) {
            return Err(Error::Capacity.into());
        }
        Ok(Some(Self {
            source: policy.source,
            index,
            capacity,
            lock: syscall(load_cell_lock(index, policy.source))?,
            type_script: syscall(load_cell_type(index, policy.source))?,
            data: cells::data::<T>(index, policy.source)?,
        }))
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn lock(&self) -> &Script {
        &self.lock
    }

    pub fn type_script(&self) -> Option<&Script> {
        self.type_script.as_ref()
    }

    pub fn data(&self) -> &T {
        &self.data
    }
}

/// Use when a Cell's contents need no typed decoding, including empty data.
#[derive(Debug, PartialEq, Eq)]
pub struct RawData(pub Vec<u8>);

impl BinaryCodec for RawData {
    type Error = Error;

    fn encode(&self) -> Vec<u8> {
        self.0.clone()
    }

    fn decode(data: &[u8]) -> Result<Self> {
        Ok(Self(data.into()))
    }
}

pub struct InputAuthority {
    lock_hash: Hash,
}

impl InputAuthority {
    /// Proves input presence; authorization semantics depend on that input's
    /// Lock. This alone does not establish a private-key signature.
    pub fn load(lock_hash: Hash) -> Result<Self> {
        cells::require_actor_input(&lock_hash)?;
        Ok(Self { lock_hash })
    }

    /// Also excludes the supplied protocol's code, including Type-hash aliases.
    pub fn load_external(lock_hash: Hash, protocol: &Script) -> Result<Self> {
        cells::require_external_actor_input(&lock_hash, protocol)?;
        Ok(Self { lock_hash })
    }

    pub fn lock_hash(&self) -> &Hash {
        &self.lock_hash
    }
}
