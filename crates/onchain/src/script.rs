use crate::{cells, syscall, Source};
use ckb_contract_core::{args, ckb_hash, token::XUDT_CODE_HASH, Error, Hash, Result};
use ckb_std::{
    ckb_types::{packed::Script, prelude::*},
    high_level::*,
};

pub fn script_hash(script: &Script) -> Hash {
    ckb_hash(script.as_slice())
}

pub fn same_code(a: &Script, b: &Script) -> bool {
    a.code_hash() == b.code_hash() && a.hash_type() == b.hash_type()
}

pub fn with_role(base: &Script, role: u8, id: &Hash, payload: &[u8]) -> Script {
    base.clone()
        .as_builder()
        .args(args::encode(role, id, payload).pack())
        .build()
}

pub fn matches_role(script: &Script, base: &Script, role: u8) -> bool {
    same_code(script, base)
        && args::RoleArgs::decode(&script.args().raw_data()).is_ok_and(|a| a.role == role)
}

/// A plain official xUDT Type. owner is a lock hash, not a public key.
pub fn plain_xudt(owner: &Hash) -> Script {
    let mut args = owner.to_vec();
    args.extend_from_slice(&[0; 4]);
    Script::new_builder()
        .code_hash(XUDT_CODE_HASH.pack())
        .hash_type(2u8)
        .args(args.pack())
        .build()
}

pub fn is_plain_xudt(script: &Script) -> bool {
    let args = script.args().raw_data();
    script.code_hash().as_slice() == XUDT_CODE_HASH
        && script.hash_type() == 2u8.into()
        && (args.len() == 32 || (args.len() == 36 && args[32..] == [0; 4]))
}

/// Construct the built-in Type ID Script. This does not turn a custom role ID
/// into Type ID; put this exact Script in the code/state Cell's type field.
pub fn type_id(id: &Hash) -> Script {
    let mut code_hash = [0; 32];
    code_hash[25..].copy_from_slice(b"TYPE_ID");
    Script::new_builder()
        .code_hash(code_hash.pack())
        .hash_type(1u8)
        .args(id.to_vec().pack())
        .build()
}

/// Resolve the actual code bytes' hash, including type-hash aliases.
/// The built-in Type ID is native consensus code and has no code Cell.
pub fn code_data_hash(script: &Script) -> Result<Option<Hash>> {
    let code_hash: Hash = script.code_hash().unpack();
    match u8::from(script.hash_type()) {
        0 | 2 | 4 => Ok(Some(code_hash)),
        1 => {
            if script.code_hash() == type_id(&[0; 32]).code_hash() {
                return Ok(None);
            }
            let mut found = None;
            for i in 0..cells::count(Source::CellDep)? {
                if syscall(load_cell_type_hash(i, Source::CellDep))? == Some(code_hash) {
                    let data_hash = syscall(load_cell_data_hash(i, Source::CellDep))?;
                    if found.is_some_and(|previous| previous != data_hash) {
                        return Err(Error::Structure);
                    }
                    found = Some(data_hash);
                }
            }
            found.map(Some).ok_or(Error::Structure)
        }
        _ => Err(Error::Encoding),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_scripts_preserve_code_and_separate_identities() {
        let base = plain_xudt(&[7; 32]);
        let state = with_role(&base, 0, &[1; 32], &[]);
        let lock = with_role(&base, 1, &[1; 32], &[]);
        assert!(same_code(&state, &lock));
        assert_ne!(script_hash(&state), script_hash(&lock));
        assert!(matches_role(&state, &base, 0));
        assert_eq!(state.args().raw_data().len(), 33);
        assert_eq!(
            with_role(&base, 2, &[1; 32], &[2; 32])
                .args()
                .raw_data()
                .len(),
            65
        );
    }

    #[test]
    fn plain_xudt_rejects_nonzero_flags_and_other_hash_types() {
        let token = plain_xudt(&[7; 32]);
        assert!(is_plain_xudt(&token));
        assert!(!is_plain_xudt(
            &token.clone().as_builder().hash_type(4u8).build()
        ));
        let mut args = token.args().raw_data().to_vec();
        args[32] = 1;
        assert!(!is_plain_xudt(
            &token.as_builder().args(args.pack()).build()
        ));
    }

    #[test]
    fn plain_xudt_accepts_32_bytes_without_normalizing_its_identity() {
        let long = plain_xudt(&[7; 32]);
        let short = long
            .clone()
            .as_builder()
            .args([7u8; 32].as_slice().pack())
            .build();
        assert!(is_plain_xudt(&short));
        assert_ne!(script_hash(&short), script_hash(&long));
        for len in [31, 33, 34, 35, 37] {
            let malformed = long
                .clone()
                .as_builder()
                .args(alloc::vec![0u8; len].pack())
                .build();
            assert!(!is_plain_xudt(&malformed));
        }
    }

    #[test]
    fn type_id_contains_the_builtin_constant_and_full_32_byte_id() {
        let script = type_id(&[7; 32]);
        assert_eq!(&script.code_hash().as_slice()[25..], b"TYPE_ID");
        assert_eq!(script.args().raw_data().as_ref(), &[7; 32]);
        assert_eq!(script.hash_type(), 1u8.into());
        assert_eq!(code_data_hash(&script), Ok(None));
    }
}
