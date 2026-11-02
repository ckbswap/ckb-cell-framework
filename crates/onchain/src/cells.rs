use crate::{script, syscall, Source};
use alloc::vec::Vec;
use ckb_contract_core::{entity_id, BinaryCodec, Error, Hash, Result};
use ckb_std::{
    ckb_types::{packed::Script, prelude::*},
    error::SysError,
    high_level::*,
};

#[derive(Clone, Copy)]
pub enum ScriptField {
    Lock,
    Type,
}

/// Only IndexOutOfBound ends iteration. Other syscall errors fail validation.
pub fn count(source: Source) -> Result<usize> {
    let mut index = 0usize;
    loop {
        match load_cell_capacity(index, source) {
            Ok(_) => index = index.checked_add(1).ok_or(Error::Arithmetic)?,
            Err(SysError::IndexOutOfBound) => return Ok(index),
            Err(_) => return Err(Error::Structure),
        }
    }
}

pub fn find(target: &Script, source: Source, field: ScriptField) -> Result<Vec<usize>> {
    let mut found = Vec::new();
    for i in 0..count(source)? {
        let matches = match field {
            ScriptField::Lock => syscall(load_cell_lock(i, source))? == *target,
            ScriptField::Type => syscall(load_cell_type(i, source))?.as_ref() == Some(target),
        };
        if matches {
            found.push(i);
        }
    }
    Ok(found)
}

pub fn exactly_one(target: &Script, source: Source, field: ScriptField) -> Result<usize> {
    let found = find(target, source, field)?;
    if found.len() != 1 {
        return Err(Error::Structure);
    }
    Ok(found[0])
}

pub fn count_role(base: &Script, role: u8, source: Source, field: ScriptField) -> Result<usize> {
    let mut count_matches = 0;
    for i in 0..count(source)? {
        let cell_script = match field {
            ScriptField::Lock => Some(syscall(load_cell_lock(i, source))?),
            ScriptField::Type => syscall(load_cell_type(i, source))?,
        };
        if cell_script
            .as_ref()
            .is_some_and(|s| script::matches_role(s, base, role))
        {
            count_matches += 1;
        }
    }
    Ok(count_matches)
}

pub fn data<T: BinaryCodec>(index: usize, source: Source) -> ::core::result::Result<T, T::Error> {
    T::decode(&syscall(load_cell_data(index, source))?)
}

pub fn action<T: BinaryCodec>(state_input: usize) -> ::core::result::Result<T, T::Error> {
    let witness = syscall(load_witness_args(state_input, Source::Input))?;
    let bytes = witness.input_type().to_opt().ok_or(Error::Encoding)?;
    T::decode(&bytes.raw_data())
}

/// Bind an action's Unix-seconds timestamp to the actual State input's since.
/// Consensus enforces maturity; the VM alone cannot prove the current chain time.
pub fn require_timestamp_since(state_input: usize, seconds: u64) -> Result<()> {
    let expected = ckb_contract_core::since::absolute_timestamp(seconds)?;
    if syscall(load_input_since(state_input, Source::Input))? != expected {
        return Err(Error::Transition);
    }
    Ok(())
}

/// Checks input presence only. Its Lock script must provide the intended authorization.
/// Public protocol locks must not be mistaken for private-key ownership.
pub fn require_actor_input(actor: &Hash) -> Result<()> {
    for i in 0..count(Source::Input)? {
        if syscall(load_cell_lock_hash(i, Source::Input))? == *actor {
            return Ok(());
        }
    }
    Err(Error::Authorization)
}

/// Require an actor input whose code differs from this protocol's public locks.
/// Resolves Type-hash aliases as well. Other locks retain their own authorization
/// semantics; this does not establish private-key ownership for arbitrary code.
pub fn require_external_actor_input(actor: &Hash, protocol: &Script) -> Result<()> {
    require_actor_input(actor)?;
    let protocol_code = script::code_data_hash(protocol)?;
    for i in 0..count(Source::Input)? {
        if syscall(load_cell_lock_hash(i, Source::Input))? == *actor
            && script::code_data_hash(&syscall(load_cell_lock(i, Source::Input))?)? == protocol_code
        {
            return Err(Error::Authorization);
        }
    }
    Ok(())
}

pub fn capacity_total(source: Source, owner: Option<&Hash>) -> Result<u64> {
    let mut total = 0u64;
    for i in 0..count(source)? {
        if let Some(owner) = owner {
            if syscall(load_cell_lock_hash(i, source))? != *owner {
                continue;
            }
        }
        total = total
            .checked_add(syscall(load_cell_capacity(i, source))?)
            .ok_or(Error::Arithmetic)?;
    }
    Ok(total)
}

pub fn transaction_fee() -> Result<u64> {
    capacity_total(Source::Input, None)?
        .checked_sub(capacity_total(Source::Output, None)?)
        .ok_or(Error::Capacity)
}

/// Require net CKB received >= expected - allowance. Caller chooses allowance explicitly.
pub fn require_payment(owner: &Hash, expected: u64, allowance: u64) -> Result<()> {
    let input = u128::from(capacity_total(Source::Input, Some(owner))?);
    let output = u128::from(capacity_total(Source::Output, Some(owner))?);
    if output + u128::from(allowance) < input + u128::from(expected) {
        return Err(Error::Capacity);
    }
    Ok(())
}

pub fn creation_id(output_index: usize) -> Result<Hash> {
    let first = syscall(load_input(0, Source::Input))?;
    let bytes = first.as_slice().try_into().map_err(|_| Error::Structure)?;
    Ok(entity_id(
        bytes,
        u64::try_from(output_index).map_err(|_| Error::Arithmetic)?,
    ))
}

/// Bind a companion input to the same previous transaction as its State input.
pub fn require_same_origin(
    state_input: usize,
    companion_input: usize,
    output_index: u32,
) -> Result<()> {
    let state = syscall(load_input(state_input, Source::Input))?.previous_output();
    let companion = syscall(load_input(companion_input, Source::Input))?.previous_output();
    if state.tx_hash() != companion.tx_hash()
        || Unpack::<u32>::unpack(&companion.index()) != output_index
    {
        return Err(Error::Structure);
    }
    Ok(())
}
