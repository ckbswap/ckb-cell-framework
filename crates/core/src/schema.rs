//! Versioned descriptions of declared transaction constraints. Expressions are
//! Rust source descriptions, not a portable executable specification.
use crate::layout::{ReserveLayout, StateLayout};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Cell,
    State,
    Reserve,
    Authority,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constraint {
    pub name: &'static str,
    pub expression: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSchema {
    pub name: &'static str,
    pub kind: FieldKind,
    pub constraints: &'static [Constraint],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextSchema {
    pub version: u16,
    pub name: &'static str,
    pub reject_standard_mint: bool,
    pub single_state: bool,
    /// Present only when using the optional single-State protocol.
    pub state_layout: Option<StateLayout>,
    /// Evaluated Reserve constraints, in Reserve field declaration order.
    pub reserve_layouts: Vec<ReserveLayout>,
    pub constraints: &'static [Constraint],
    pub fields: &'static [FieldSchema],
}
