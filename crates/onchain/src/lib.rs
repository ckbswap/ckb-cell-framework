//! CKB syscall adapters and reusable transaction validation, with explicit policies.
#![no_std]
extern crate alloc;

pub mod cells;
pub mod context;
pub mod patterns;
pub mod script;
pub mod state;
pub mod token;

pub use ckb_contract_core as core;
pub use ckb_std;
pub use ckb_std::{ckb_constants::Source, ckb_types};

use ckb_contract_core::{Error, Result};
use ckb_std::error::SysError;

pub fn syscall<T>(value: ::core::result::Result<T, SysError>) -> Result<T> {
    value.map_err(|_| Error::Structure)
}
