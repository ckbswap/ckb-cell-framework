use ckb_contract_core::{schema::FieldKind, BinaryCodec, Error};
use ckb_contract_onchain::{
    ckb_types::packed::Script,
    context::{Cell, CellPolicy, Cells, Context, InputAuthority, RawData, TypeConstraint},
    Source,
};

#[derive(Debug, PartialEq)]
enum AppError {
    Framework(Error),
    InvalidData,
}

impl From<Error> for AppError {
    fn from(error: Error) -> Self {
        Self::Framework(error)
    }
}

// No Clone, StateTransition, Action or protocol layout is required.
struct Value(u8);
impl BinaryCodec for Value {
    type Error = AppError;
    fn encode(&self) -> Vec<u8> {
        vec![self.0]
    }
    fn decode(bytes: &[u8]) -> Result<Self, AppError> {
        match bytes {
            [value] => Ok(Self(*value)),
            _ => Err(AppError::InvalidData),
        }
    }
}

struct Params<'a> {
    script: &'a Script,
}

#[derive(Cells)]
#[cells(args = Params<'a>, error = AppError,
    constraint = first.data().0 <= second.data().0 && first.index() != second.index())]
struct TwoCells {
    #[cell(source = Source::Input, index = 0, type_script = Some(args.script))]
    first: Cell<Value>,
    #[cell(source = Source::Input, index = 1, type_script = Some(args.script),
        constraint = second.data().0 > 0)]
    second: Cell<Value>,
    #[cell(source = Source::CellDep, index = 0, type_script = None)]
    config: Cell<RawData>,
    #[authority(lock_hash = [1; 32], protocol = args.script)]
    owner: InputAuthority,
}

#[derive(Cells)]
struct AuthorityOnly {
    #[authority(lock_hash = [1; 32])]
    owner: InputAuthority,
}

#[test]
fn generic_context_accepts_multiple_plain_cells_and_borrowed_parameters() {
    let _: fn(&Params<'_>) -> Result<Context<TwoCells>, AppError> = Context::<TwoCells>::load;
    let schema = TwoCells::schema();
    assert_eq!(schema.state_layout, None);
    assert!(schema.reserve_layouts.is_empty());
    assert!(!schema.single_state);
    assert_eq!(schema.fields.len(), 4);
    assert!(schema.fields[..3].iter().all(|f| f.kind == FieldKind::Cell));
    assert_eq!(schema.constraints[0].name, "constraint");
    let _read = |ctx: Context<TwoCells>| {
        (
            ctx.cells.first.index(),
            ctx.cells.second.index(),
            ctx.cells.config.source(),
            *ctx.cells.owner.lock_hash(),
        )
    };
    let _: fn(&()) -> Result<Context<AuthorityOnly>, Error> = Context::<AuthorityOnly>::load;
    let _read_authority = |ctx: Context<AuthorityOnly>| *ctx.cells.owner.lock_hash();
}

#[test]
fn missing_selector_fails_without_syscalls_and_preserves_custom_error() {
    let result = Cell::<Value>::load(CellPolicy {
        source: Source::Input,
        index: None,
        lock: None,
        type_script: TypeConstraint::Any,
        capacity: None,
    });
    assert!(matches!(result, Err(AppError::Framework(Error::Structure))));
}
