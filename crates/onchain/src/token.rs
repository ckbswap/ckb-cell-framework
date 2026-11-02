use crate::{
    cells::{self, ScriptField},
    script, syscall, Source,
};
use alloc::vec::Vec;
use ckb_contract_core::{
    check_supply,
    token::{decode_amount, AmountEncoding, SUDT_CODE_HASH, XUDT_CODE_HASH},
    Error, Hash, Result,
};
use ckb_std::{ckb_types::packed::Script, high_level::*};

pub fn total(
    type_hash: &Hash,
    source: Source,
    owner: Option<&Hash>,
    encoding: AmountEncoding,
) -> Result<u128> {
    let mut total = 0u128;
    for i in 0..cells::count(source)? {
        if syscall(load_cell_type_hash(i, source))?.as_ref() != Some(type_hash) {
            continue;
        }
        if let Some(owner) = owner {
            if syscall(load_cell_lock_hash(i, source))? != *owner {
                continue;
            }
        }
        let amount = decode_amount(&syscall(load_cell_data(i, source))?, encoding)?;
        total = total.checked_add(amount).ok_or(Error::Arithmetic)?;
    }
    Ok(total)
}

pub fn require_supply_change(type_hash: &Hash, mint: u128, burn: u128) -> Result<()> {
    check_supply(
        total(type_hash, Source::Input, None, AmountEncoding::Prefix16)?,
        total(type_hash, Source::Output, None, AmountEncoding::Prefix16)?,
        mint,
        burn,
    )
}

/// Reject positive net supply changes for every official xUDT/sUDT output Type.
/// Exemptions MUST be full Type hashes and need their own supply/actor accounting.
/// This is not a guard for unknown/custom token binaries.
pub fn reject_standard_mint(exempt_types: &[Hash]) -> Result<()> {
    let mut seen: Vec<Hash> = Vec::new();
    for i in 0..cells::count(Source::Output)? {
        let Some(token) = syscall(load_cell_type(i, Source::Output))? else {
            continue;
        };
        let type_hash = script::script_hash(&token);
        if exempt_types.contains(&type_hash) || seen.contains(&type_hash) {
            continue;
        }
        seen.push(type_hash);
        let code_hash = script::code_data_hash(&token)?;
        if (code_hash == Some(XUDT_CODE_HASH) || code_hash == Some(SUDT_CODE_HASH))
            && total(&type_hash, Source::Output, None, AmountEncoding::Prefix16)?
                > total(&type_hash, Source::Input, None, AmountEncoding::Prefix16)?
        {
            return Err(Error::Token);
        }
    }
    Ok(())
}

pub struct ReserveSpec<'a> {
    pub lock: &'a Script,
    pub token_type: &'a Script,
    pub amount: u128,
    pub capacity: u64,
}

impl ReserveSpec<'_> {
    /// Returns the exact matching Cell index; callers enforce output position and origin.
    pub fn verify(&self, source: Source) -> Result<usize> {
        let i = cells::exactly_one(self.lock, source, ScriptField::Lock)?;
        if !script::is_plain_xudt(self.token_type)
            || syscall(load_cell_type(i, source))?.as_ref() != Some(self.token_type)
            || decode_amount(
                &syscall(load_cell_data(i, source))?,
                AmountEncoding::Exact16,
            )? != self.amount
        {
            return Err(Error::Token);
        }
        if syscall(load_cell_capacity(i, source))? != self.capacity {
            return Err(Error::Capacity);
        }
        Ok(i)
    }
}
