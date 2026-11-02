//! Shared, network-independent Cell layout. Code identity and asset Types are
//! resolved separately; this describes positions, roles and fixed capacity.
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateLayout {
    pub role: u8,
    pub lock_role: u8,
    pub capacity: u64,
    pub output: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReserveLayout {
    pub role: u8,
    pub capacity: u64,
    pub output: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionLayout<'a> {
    pub state: StateLayout,
    pub reserves: &'a [ReserveLayout],
}

impl TransactionLayout<'_> {
    pub fn validate(&self) -> Result<()> {
        validate_layout(
            &[self.state.role, self.state.lock_role],
            &[self.state.output],
        )?;
        for (index, reserve) in self.reserves.iter().enumerate() {
            validate_layout(
                &[self.state.role, self.state.lock_role, reserve.role],
                &[self.state.output, reserve.output],
            )?;
            for previous in &self.reserves[..index] {
                validate_layout(
                    &[previous.role, reserve.role],
                    &[previous.output, reserve.output],
                )?;
            }
        }
        Ok(())
    }
}

/// Validate evaluated values rather than constant names. OutPoint indices are u32.
pub fn validate_layout(roles: &[u8], outputs: &[usize]) -> Result<()> {
    for (index, role) in roles.iter().enumerate() {
        if roles[..index].contains(role) {
            return Err(Error::Encoding);
        }
    }
    for (index, output) in outputs.iter().enumerate() {
        if *output > u32::MAX as usize || outputs[..index].contains(output) {
            return Err(Error::Structure);
        }
    }
    Ok(())
}
