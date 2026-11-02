//! Host-side v1 interface export. Rust callbacks remain descriptive source;
//! portable wire shapes, layout values and operation bindings are explicit.
use ckb_contract_core::{
    layout::TransactionLayout,
    schema::{ContextSchema, FieldKind},
    wire::{EncodingSchema, WireField, WireType},
};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Create,
    Update,
    Close,
}

pub struct Instruction<'a> {
    pub name: &'a str,
    pub phase: Phase,
    /// None for creation; otherwise the exact Action variant name.
    pub action: Option<&'a str>,
    /// A 32-byte Lock Hash field path in State, not a signature algorithm.
    pub authority: &'a str,
    /// A u64 field path in the selected Action, bound to State input since.
    pub timestamp: Option<&'a str>,
}

/// Explicit output-field bindings for an application that uses generic Cell
/// contexts but chooses to publish this optional State-protocol interface.
/// These names are checked, but arbitrary Rust constraints are not evaluated.
pub struct ContextBindings<'a> {
    pub state_output: &'a str,
    pub reserve_outputs: &'a [&'a str],
}

pub struct Interface<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub state: EncodingSchema,
    pub action: EncodingSchema,
    pub layout: TransactionLayout<'a>,
    pub context: ContextSchema,
    pub bindings: Option<ContextBindings<'a>>,
    pub instructions: &'a [Instruction<'a>],
}

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Layout(ckb_contract_core::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "invalid interface: {message}"),
            Self::Layout(error) => write!(f, "invalid layout: {error:?}"),
            Self::Json(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// Stable pretty JSON with a trailing newline. u64 capacities are decimal
/// strings so JavaScript clients cannot silently lose integer precision.
pub fn export(interface: &Interface<'_>) -> Result<String> {
    interface.layout.validate().map_err(Error::Layout)?;
    if interface.name.is_empty()
        || interface.version.is_empty()
        || interface.instructions.is_empty()
    {
        return Err(Error::Invalid(
            "name, version and instructions are required",
        ));
    }
    let (state, _) = encoding(interface.state)?;
    let (action, _) = encoding(interface.action)?;
    let state_fields = match interface.state.body {
        WireType::Struct { fields, .. } => fields,
        _ => return Err(Error::Invalid("State must be a struct")),
    };
    let variants = match interface.action.body {
        WireType::Enum { variants, .. } => variants,
        _ => return Err(Error::Invalid("Action must be a tagged enum")),
    };
    let states: Vec<_> = interface
        .context
        .fields
        .iter()
        .filter(|field| field.kind == FieldKind::State)
        .collect();
    let reserves: Vec<_> = interface
        .context
        .fields
        .iter()
        .filter(|field| field.kind == FieldKind::Reserve)
        .collect();
    if interface.context.version != 1 {
        return Err(Error::Invalid("unsupported context version"));
    }
    let (state_name, reserve_names, layout_verification) =
        if let Some(bindings) = &interface.bindings {
            if interface.context.state_layout.is_some()
                || !states.is_empty()
                || !reserves.is_empty()
                || !interface.context.reserve_layouts.is_empty()
                || bindings.reserve_outputs.len() != interface.layout.reserves.len()
            {
                return Err(Error::Invalid(
                    "explicit bindings require a generic context and the same reserve count",
                ));
            }
            let names: Vec<_> = ::core::iter::once(bindings.state_output)
                .chain(bindings.reserve_outputs.iter().copied())
                .collect();
            unique(names.iter().copied())?;
            for name in names {
                if !interface
                    .context
                    .fields
                    .iter()
                    .any(|field| field.name == name && field.kind == FieldKind::Cell)
                {
                    return Err(Error::Invalid(
                        "context binding must reference a declared Cell field",
                    ));
                }
            }
            (
                bindings.state_output,
                bindings.reserve_outputs.to_vec(),
                "explicit_context_bindings",
            )
        } else {
            if states.len() != 1 || reserves.len() != interface.layout.reserves.len() {
                return Err(Error::Invalid(
                    "context and layout must describe one State and the same reserve count",
                ));
            }
            if interface.context.state_layout != Some(interface.layout.state)
                || interface.context.reserve_layouts.as_slice() != interface.layout.reserves
            {
                return Err(Error::Invalid("context and interface layout values differ"));
            }
            (
                states[0].name,
                reserves.iter().map(|field| field.name).collect(),
                "derived_state_pattern",
            )
        };
    unique(interface.context.fields.iter().map(|field| field.name))?;
    unique(
        interface
            .instructions
            .iter()
            .map(|instruction| instruction.name),
    )?;
    let mut instructions = Vec::new();
    for instruction in interface.instructions {
        if lookup(state_fields, instruction.authority) != Some(WireType::Bytes(32)) {
            return Err(Error::Invalid(
                "authority must reference a 32-byte State field",
            ));
        }
        let fields = match (instruction.phase, instruction.action) {
            (Phase::Create, None) => &[][..],
            (Phase::Update | Phase::Close, Some(action)) => {
                variants
                    .iter()
                    .find(|variant| variant.name == action)
                    .ok_or(Error::Invalid("instruction references an unknown Action"))?
                    .fields
            }
            _ => {
                return Err(Error::Invalid(
                    "creation has no input Action; update and close require an Action",
                ))
            }
        };
        let since = match instruction.timestamp {
            Some(field) => {
                if instruction.phase != Phase::Update
                    || lookup(fields, field) != Some(WireType::U64)
                {
                    return Err(Error::Invalid(
                        "timestamp must reference a u64 update Action field",
                    ));
                }
                json!({"metric": "absolute_timestamp", "unit": "unix_seconds", "input": "actual_state_input", "action_field": field})
            }
            None => Value::Null,
        };
        let phase = match instruction.phase {
            Phase::Create => "create",
            Phase::Update => "update",
            Phase::Close => "close",
        };
        instructions.push(json!({
            "name": instruction.name, "lifecycle": phase, "action": instruction.action,
            "authority": {"kind": "input_lock_hash", "state_field": instruction.authority}, "since": since,
        }));
    }
    if variants.iter().any(|variant| {
        !interface
            .instructions
            .iter()
            .any(|instruction| instruction.action == Some(variant.name))
    }) {
        return Err(Error::Invalid(
            "every Action variant needs an instruction binding",
        ));
    }
    let layout = interface.layout;
    let reserve_layouts: Vec<_> = layout.reserves.iter().zip(&reserve_names).map(|(reserve, name)| json!({
        "name": name, "role": reserve.role, "capacity": reserve.capacity.to_string(), "output": reserve.output,
        "amount_encoding": {"kind": "u128", "endianness": "little", "byte_length": 16},
    })).collect();
    let value = json!({
        "schema_version": 1, "name": interface.name, "version": interface.version,
        "state": state, "action": action,
        "layout": {
            "capacity_unit": "shannon",
            "script_args": {"format": "role_u8_id_bytes32", "payload_hex": ""},
            "creation_id": {"hash": "blake2b_256", "personalization": "ckb-default-hash",
                "preimage": ["first_cell_input_molecule", "state_output_index_u64_le"]},
            "state": {"name": state_name, "role": layout.state.role, "lock_role": layout.state.lock_role,
                "capacity": layout.state.capacity.to_string(), "output": layout.state.output},
            "reserves": reserve_layouts,
        },
        "witness": {"source": "input", "index": "actual_state_input", "field": "input_type"},
        "instructions": instructions,
        "context": context_value(&interface.context),
        "layout_verification": layout_verification,
    });
    Ok(serde_json::to_string_pretty(&value).map_err(Error::Json)? + "\n")
}

/// Export generic Context declarations without inventing a State, Action,
/// role-args format, output layout or witness convention. Expressions describe
/// Rust checks; they are not executable cross-language constraints.
pub fn export_context(context: &ContextSchema) -> Result<String> {
    if context.version != 1 || context.name.is_empty() {
        return Err(Error::Invalid("context version and name are required"));
    }
    unique(context.fields.iter().map(|field| field.name))?;
    serde_json::to_string_pretty(&context_value(context))
        .map(|json| json + "\n")
        .map_err(Error::Json)
}

fn context_value(context: &ContextSchema) -> Value {
    let constraints =
        |values: &[ckb_contract_core::schema::Constraint]| -> Vec<Value> {
            values.iter().map(|constraint| json!({
            "name": constraint.name, "language": "rust", "source": constraint.expression,
        })).collect()
        };
    let fields: Vec<_> = context
        .fields
        .iter()
        .map(|field| {
            let kind = match field.kind {
                FieldKind::Cell => "cell",
                FieldKind::State => "state",
                FieldKind::Reserve => "reserve",
                FieldKind::Authority => "authority",
            };
            json!({"name": field.name, "kind": kind, "constraints": constraints(field.constraints)})
        })
        .collect();
    json!({
        "version": context.version, "name": context.name,
        "reject_standard_mint": context.reject_standard_mint, "single_state": context.single_state,
        "constraints": constraints(context.constraints), "fields": fields,
    })
}

pub fn write(interface: &Interface<'_>, path: &Path) -> Result<()> {
    let json = export(interface)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(Error::Io)?;
    }
    fs::write(path, json).map_err(Error::Io)
}

fn unique<'a>(names: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = HashSet::new();
    for name in names {
        if name.is_empty() || !seen.insert(name) {
            return Err(Error::Invalid("names must be nonempty and unique"));
        }
    }
    Ok(())
}

fn lookup(fields: &[WireField], path: &str) -> Option<WireType> {
    let (head, tail) = path
        .split_once('.')
        .map_or((path, None), |(head, tail)| (head, Some(tail)));
    let field = fields.iter().find(|field| field.name == head)?;
    match tail {
        None => Some(field.ty),
        Some(tail) => match field.ty {
            WireType::Struct { fields, .. } => lookup(fields, tail),
            _ => None,
        },
    }
}

fn encoding(schema: EncodingSchema) -> Result<(Value, Option<usize>)> {
    let (body, len) = wire(schema.body, 0)?;
    let total = len
        .map(|len| {
            len.checked_add(schema.prefix.len())
                .ok_or(Error::Invalid("wire byte length overflow"))
        })
        .transpose()?;
    let prefix: String = schema
        .prefix
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((
        json!({"prefix_hex": prefix, "endianness": "little", "strict": true,
        "body_offset": schema.prefix.len(), "byte_length": total, "body": body}),
        total,
    ))
}

fn fields(
    fields: &[WireField],
    offset: usize,
    depth: usize,
) -> Result<(Vec<Value>, Option<usize>)> {
    unique(fields.iter().map(|field| field.name))?;
    let mut position = Some(offset);
    let mut result = Vec::new();
    for field in fields {
        let (ty, len) = wire(field.ty, depth + 1)?;
        result.push(json!({"name": field.name, "offset": position, "type": ty}));
        position = position
            .zip(len)
            .map(|(offset, len)| {
                offset
                    .checked_add(len)
                    .ok_or(Error::Invalid("wire byte length overflow"))
            })
            .transpose()?;
    }
    Ok((result, position))
}

fn wire(ty: WireType, depth: usize) -> Result<(Value, Option<usize>)> {
    if depth > 64 {
        return Err(Error::Invalid("wire schema nesting exceeds 64 levels"));
    }
    let (kind, len) = match ty {
        WireType::U8 => ("u8", 1),
        WireType::U32 => ("u32", 4),
        WireType::U64 => ("u64", 8),
        WireType::U128 => ("u128", 16),
        WireType::Bool => ("bool", 1),
        WireType::Bytes(len) => ("bytes", len),
        WireType::Struct {
            name,
            fields: members,
        } => {
            let (fields, len) = fields(members, 0, depth)?;
            return Ok((
                json!({"kind":"struct", "name": name, "fields": fields, "byte_length": len}),
                len,
            ));
        }
        WireType::Enum { name, variants } => {
            unique(variants.iter().map(|variant| variant.name))?;
            if variants.is_empty() {
                return Err(Error::Invalid("an enum needs variants"));
            }
            let mut tags = HashSet::new();
            let mut output = Vec::new();
            let mut lengths = Vec::new();
            for variant in variants {
                if !tags.insert(variant.tag) {
                    return Err(Error::Invalid("duplicate enum tag"));
                }
                let (fields, len) = fields(variant.fields, 1, depth)?;
                lengths.push(len);
                output.push(json!({"name": variant.name, "tag": variant.tag, "fields": fields, "byte_length": len}));
            }
            let len = lengths[0].filter(|first| lengths.iter().all(|len| *len == Some(*first)));
            return Ok((
                json!({"kind":"enum", "name": name, "tag_type":"u8", "variants":output, "byte_length": len}),
                len,
            ));
        }
    };
    Ok((json!({"kind": kind, "byte_length": len}), Some(len)))
}
