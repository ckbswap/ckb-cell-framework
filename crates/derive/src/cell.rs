//! Generic contexts need neither a distinguished State nor role-based args.
use super::{required, Field};
use proc_macro2::TokenStream as Tokens;
use quote::quote;
use syn::{Expr, GenericArgument, PathArguments, Type};

fn cell_type(ty: &Type) -> Option<(&Type, bool)> {
    if let Type::Path(path) = ty {
        let segment = path.path.segments.last()?;
        if let PathArguments::AngleBracketed(args) = &segment.arguments {
            if args.args.len() != 1 {
                return None;
            }
            if let Some(GenericArgument::Type(inner)) = args.args.first() {
                if segment.ident == "Cell" {
                    return Some((inner, false));
                }
                if segment.ident == "Option" {
                    let (data, optional) = cell_type(inner)?;
                    if !optional {
                        return Some((data, true));
                    }
                }
            }
        }
    }
    None
}

fn data_type(field: &Field) -> syn::Result<&Type> {
    cell_type(&field.ty).map(|(data, _)| data).ok_or_else(|| {
        syn::Error::new_spanned(
            &field.ty,
            "cell field must have type Cell<Data> or Option<Cell<Data>>",
        )
    })
}

pub(super) fn optional_constraint(field: &Field) -> Option<Tokens> {
    (field.kind == "cell" && cell_type(&field.ty).is_some_and(|(_, optional)| optional)).then(
        || {
            quote!(::ckb_contract_core::schema::Constraint {
                name: "optional",
                expression: "true"
            })
        },
    )
}

pub(super) fn load(field: &Field) -> syn::Result<Tokens> {
    data_type(field)?;
    let loader = if cell_type(&field.ty).unwrap().1 {
        quote!(load_optional)
    } else {
        quote!(load)
    };
    let ident = &field.ident;
    let ty = &field.ty;
    let source = required(field, "source")?;
    if !["index", "lock", "type_script"]
        .iter()
        .any(|key| field.args.contains_key(*key))
    {
        return Err(syn::Error::new_spanned(
            ident,
            "cell requires an index, lock or type_script selector",
        ));
    }
    let option = |key| {
        field
            .args
            .get(key)
            .map_or_else(|| quote!(None), |e| quote!(Some(#e)))
    };
    let index = option("index");
    let lock = option("lock");
    let capacity = option("capacity");
    let type_script = field.args.get("type_script").map_or_else(
        || quote!(::ckb_contract_onchain::context::TypeConstraint::Any),
        |e| quote!(::ckb_contract_onchain::context::TypeConstraint::from(#e)),
    );
    let check = field.args.get("constraint").map(|expr| {
        quote! {
            if !(#expr) { return Err(::ckb_contract_core::Error::Structure.into()); }
        }
    });
    Ok(quote! {
        let #ident: #ty = ::ckb_contract_onchain::context::Cell::#loader(
            ::ckb_contract_onchain::context::CellPolicy {
                source: #source, index: #index, lock: #lock, type_script: #type_script, capacity: #capacity,
            }
        )?;
        #check
    })
}

pub(super) fn expand(
    name: &syn::Ident,
    fields: &[Field],
    args_type: Option<Type>,
    error_type: Option<Type>,
    check: Option<Expr>,
    no_mint: bool,
    single_state: bool,
) -> syn::Result<Tokens> {
    if fields.is_empty() || single_state || fields.iter().any(|f| f.kind == "reserve") {
        return Err(syn::Error::new_spanned(name, "generic contexts need fields; reserve and single_state require the optional State pattern"));
    }
    if fields.iter().any(|f| f.ident == "args") {
        return Err(syn::Error::new_spanned(
            name,
            "args is reserved for context parameters",
        ));
    }
    let args_type = args_type.map_or_else(|| quote!(()), |ty| quote!(#ty));
    let error_type = match error_type {
        Some(ty) => quote!(#ty),
        None => match fields.iter().find(|f| f.kind == "cell") {
            Some(field) => {
                let ty = data_type(field)?;
                quote!(<#ty as ::ckb_contract_core::BinaryCodec>::Error)
            }
            None => quote!(::ckb_contract_core::Error),
        },
    };
    let mut loads = Vec::new();
    let mut descriptors = Vec::new();
    for field in fields {
        let ident = &field.ident;
        let ty = &field.ty;
        let name = ident.to_string();
        let kind = if field.kind == "cell" {
            loads.push(load(field)?);
            quote!(Cell)
        } else {
            let hash = required(field, "lock_hash")?;
            let call = match field.args.get("protocol") {
                Some(protocol) => {
                    quote!(::ckb_contract_onchain::context::InputAuthority::load_external(#hash, #protocol))
                }
                None => quote!(::ckb_contract_onchain::context::InputAuthority::load(#hash)),
            };
            loads.push(quote!(let #ident: #ty = #call?;));
            quote!(Authority)
        };
        let constraints = field.args.iter().map(|(key, value)| quote!(
            ::ckb_contract_core::schema::Constraint { name: #key, expression: stringify!(#value) }
        )).chain(optional_constraint(field));
        descriptors.push(quote!(::ckb_contract_core::schema::FieldSchema {
            name: #name, kind: ::ckb_contract_core::schema::FieldKind::#kind, constraints: &[#(#constraints),*],
        }));
    }
    let guard = no_mint.then(|| quote!(::ckb_contract_onchain::token::reject_standard_mint(&[])?;));
    let check_code = check.as_ref().map(|expr| {
        quote! {
            if !(#expr) { return Err(::ckb_contract_core::Error::Transition.into()); }
        }
    });
    let context_constraints = check.into_iter().map(|expr| {
        quote!(::ckb_contract_core::schema::Constraint {
            name: "constraint",
            expression: stringify!(#expr),
        })
    });
    let names = fields.iter().map(|f| &f.ident);
    Ok(quote! {
        impl ::ckb_contract_onchain::context::Cells for #name {
            type Error = #error_type;
            type Args<'a> = #args_type;

            fn load(args: &Self::Args<'_>) -> ::core::result::Result<Self, Self::Error> {
                let _ = args;
                #guard
                #(#loads)*
                #check_code
                Ok(Self { #(#names),* })
            }

            fn schema() -> ::ckb_contract_core::schema::ContextSchema {
                ::ckb_contract_core::schema::ContextSchema {
                    version: 1, name: stringify!(#name), reject_standard_mint: #no_mint,
                    single_state: false, state_layout: None, reserve_layouts: ::core::default::Default::default(),
                    constraints: &[#(#context_constraints),*], fields: &[#(#descriptors),*],
                }
            }
        }
    })
}
