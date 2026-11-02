//! Derive checked CKB transaction contexts. No code is evaluated by the macro;
//! expressions become ordinary Rust in the generated load implementation.
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote, ToTokens};
use std::collections::BTreeMap;
use syn::{
    parse_macro_input, Data, DeriveInput, Expr, Fields, GenericArgument, LitStr, PathArguments,
    Type,
};

mod cell;
mod codec;

#[proc_macro_derive(CellCodec, attributes(codec))]
pub fn cell_codec(input: TokenStream) -> TokenStream {
    codec::expand(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(Cells, attributes(cells, cell, state, reserve, authority))]
pub fn derive_cells(input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

struct Field {
    ident: syn::Ident,
    ty: Type,
    kind: String,
    args: BTreeMap<String, Expr>,
}

fn required<'a>(field: &'a Field, name: &str) -> syn::Result<&'a Expr> {
    field.args.get(name).ok_or_else(|| {
        syn::Error::new_spanned(&field.ident, format!("missing `{name}` constraint"))
    })
}

// Mixing shared layout and individual attributes would create competing policies.
fn layout_values(field: &Field, names: &[&str]) -> syn::Result<Vec<Tokens>> {
    if let Some(layout) = field.args.get("layout") {
        if names.iter().any(|name| field.args.contains_key(*name)) {
            return Err(syn::Error::new_spanned(
                layout,
                "layout cannot be combined with role, lock_role, capacity or output",
            ));
        }
        let local = format_ident!(
            "__ckb_layout_{}",
            field.ident,
            span = proc_macro2::Span::mixed_site()
        );
        Ok(names
            .iter()
            .map(|name| {
                let member = syn::Ident::new(name, proc_macro2::Span::call_site());
                quote!(#local.#member)
            })
            .collect())
    } else {
        names
            .iter()
            .map(|name| required(field, name).map(ToTokens::to_token_stream))
            .collect()
    }
}

fn state_type(field: &Field) -> syn::Result<&Type> {
    if let Type::Path(path) = &field.ty {
        if let Some(segment) = path.path.segments.last() {
            if segment.ident == "StateCell" {
                if let PathArguments::AngleBracketed(args) = &segment.arguments {
                    if args.args.len() == 1 {
                        if let Some(GenericArgument::Type(ty)) = args.args.first() {
                            return Ok(ty);
                        }
                    }
                }
            }
        }
    }
    Err(syn::Error::new_spanned(
        &field.ty,
        "state field must have type StateCell<State>",
    ))
}

fn expand(input: DeriveInput) -> syn::Result<Tokens> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "generic contexts are not supported; use concrete Cell data types and context args",
        ));
    }
    let mut no_mint = false;
    let mut single_state = false;
    let mut args_type = None;
    let mut error_type = None;
    let mut context_check = None;
    for attr in &input.attrs {
        if ["cell", "state", "reserve", "authority"]
            .iter()
            .any(|name| attr.path().is_ident(name))
        {
            return Err(syn::Error::new_spanned(
                attr,
                "field constraints belong on fields",
            ));
        }
        if attr.path().is_ident("cells") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("no_mint") && !no_mint {
                    no_mint = true;
                } else if meta.path.is_ident("single_state") && !single_state {
                    single_state = true;
                } else if meta.path.is_ident("args") && args_type.is_none() {
                    args_type = Some(meta.value()?.parse::<Type>()?);
                } else if meta.path.is_ident("error") && error_type.is_none() {
                    error_type = Some(meta.value()?.parse::<Type>()?);
                } else if meta.path.is_ident("constraint") && context_check.is_none() {
                    context_check = Some(meta.value()?.parse::<Expr>()?);
                } else {
                    return Err(meta.error("unknown or duplicate context policy"));
                }
                Ok(())
            })?;
        }
    }
    let named = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields,
            fields => {
                return Err(syn::Error::new_spanned(
                    fields,
                    "Cells requires named fields",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Cells requires a struct",
            ))
        }
    };
    let mut fields = Vec::new();
    for field in named.named {
        if field.attrs.iter().any(|attr| attr.path().is_ident("cells")) {
            return Err(syn::Error::new_spanned(
                &field,
                "context policies belong on the struct",
            ));
        }
        let attrs: Vec<_> = field
            .attrs
            .iter()
            .filter(|a| {
                ["cell", "state", "reserve", "authority"]
                    .iter()
                    .any(|name| a.path().is_ident(name))
            })
            .collect();
        if attrs.len() != 1 {
            return Err(syn::Error::new_spanned(
                &field,
                "each field requires exactly one cell, state, reserve or authority constraint",
            ));
        }
        let attr = attrs[0];
        let kind = attr
            .path()
            .get_ident()
            .expect("checked attribute")
            .to_string();
        let allowed: &[&str] = match kind.as_str() {
            "cell" => &[
                "source",
                "index",
                "lock",
                "type_script",
                "capacity",
                "constraint",
            ],
            "state" => &[
                "layout",
                "role",
                "lock_role",
                "capacity",
                "output",
                "mode",
                "initial",
                "close",
                "close_action",
                "update_timestamp",
            ],
            "reserve" => &[
                "state", "role", "asset", "capacity", "output", "before", "after", "layout",
            ],
            _ => &["lock_hash", "protocol"],
        };
        let mut args = BTreeMap::new();
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .ok_or_else(|| meta.error("expected constraint name"))?
                .to_string();
            if !allowed.contains(&key.as_str()) || args.contains_key(&key) {
                return Err(meta.error("unknown or duplicate field constraint"));
            }
            args.insert(key, meta.value()?.parse::<Expr>()?);
            Ok(())
        })?;
        fields.push(Field {
            ident: field.ident.expect("named field"),
            ty: field.ty,
            kind,
            args,
        });
    }
    let states: Vec<_> = fields.iter().filter(|f| f.kind == "state").collect();
    if states.is_empty() {
        return cell::expand(
            &input.ident,
            &fields,
            args_type,
            error_type,
            context_check,
            no_mint,
            single_state,
        );
    }
    if args_type.is_some() || error_type.is_some() {
        return Err(syn::Error::new_spanned(&input.ident, "State patterns use RoleEnvironment and the State codec error; custom args/error apply to generic Cell contexts"));
    }
    if states.len() != 1 {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "the State pattern requires exactly one state field; use generic Cell fields for multiple states",
        ));
    }
    if fields.first().is_none_or(|f| f.kind != "state") {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "state must be declared before dependent fields",
        ));
    }
    let state = states[0];
    let state_ident = &state.ident;
    let protocol = state_type(state)?;
    let mode_expr = required(state, "mode")?;
    let mode: LitStr = syn::parse2(mode_expr.to_token_stream())?;
    let lifecycle = match mode.value().as_str() {
        "create" => quote!(Create),
        "update" => quote!(Update),
        "close" => quote!(Close),
        "persist" => quote!(Persist),
        "any" => quote!(Any),
        _ => {
            return Err(syn::Error::new_spanned(
                mode,
                "mode must be create, update, close, persist or any",
            ))
        }
    };
    let creates = matches!(mode.value().as_str(), "create" | "persist" | "any");
    let closes = matches!(mode.value().as_str(), "close" | "any");
    if !matches!(mode.value().as_str(), "update" | "persist" | "any")
        && state.args.contains_key("update_timestamp")
    {
        return Err(syn::Error::new_spanned(
            &state.ident,
            "update_timestamp requires an update lifecycle",
        ));
    }
    if creates {
        required(state, "initial")?;
    }
    if closes {
        required(state, "close")?;
        required(state, "close_action")?;
    }
    if !creates && state.args.contains_key("initial") {
        return Err(syn::Error::new_spanned(
            &state.ident,
            "initial is only valid for a creation lifecycle",
        ));
    }
    if !closes && (state.args.contains_key("close") || state.args.contains_key("close_action")) {
        return Err(syn::Error::new_spanned(
            &state.ident,
            "close policies require close or any mode",
        ));
    }
    let env_ident = syn::Ident::new("__ckb_context_env", proc_macro2::Span::mixed_site());
    let mut loads = Vec::new();
    let mut descriptors = Vec::new();
    let mut roles = Vec::new();
    let mut outputs = Vec::new();
    let mut layouts = Vec::new();
    let mut state_layout = None;
    let mut reserve_layouts = Vec::new();
    for field in &fields {
        let ident = &field.ident;
        let ty = &field.ty;
        let name = ident.to_string();
        if let Some(layout) = field.args.get("layout") {
            let local = format_ident!(
                "__ckb_layout_{}",
                ident,
                span = proc_macro2::Span::mixed_site()
            );
            let layout_type = if field.kind == "state" {
                quote!(StateLayout)
            } else {
                quote!(ReserveLayout)
            };
            layouts.push(quote!(let #local: ::ckb_contract_core::layout::#layout_type = #layout;));
        }
        let constraints: Vec<_> = field.args.iter().map(|(key, value)| quote!(::ckb_contract_core::schema::Constraint { name: #key, expression: stringify!(#value) })).chain(cell::optional_constraint(field)).collect();
        let kind = match field.kind.as_str() {
            "cell" => {
                loads.push(cell::load(field)?);
                quote!(Cell)
            }
            "state" => {
                let values = layout_values(field, &["role", "lock_role", "capacity", "output"])?;
                let (role, lock_role, capacity, output) =
                    (&values[0], &values[1], &values[2], &values[3]);
                state_layout = Some(quote!(::ckb_contract_core::layout::StateLayout {
                    role: #role, lock_role: #lock_role, capacity: #capacity, output: #output,
                }));
                let initial = field
                    .args
                    .get("initial")
                    .map_or_else(|| quote!(None), |e| quote!(Some(#e)));
                let close = field
                    .args
                    .get("close")
                    .map_or_else(|| quote!(None), |e| quote!(Some(#e)));
                let close_action = field
                    .args
                    .get("close_action")
                    .map_or_else(|| quote!(None), |e| quote!(Some(#e)));
                let timestamp_check = field
                    .args
                    .get("update_timestamp")
                    .map(|callback| quote!(#ident.require_update_timestamp(#callback)?;));
                roles.extend([role.clone(), lock_role.clone()]);
                outputs.push(output.clone());
                loads.push(quote! {
                    let #ident: #ty = ::ckb_contract_onchain::patterns::StateCell::load_and_verify(#env_ident,
                        ::ckb_contract_onchain::patterns::StatePolicy {
                            role: #role, lock_role: #lock_role, capacity: #capacity, output: #output,
                            lifecycle: ::ckb_contract_onchain::patterns::Lifecycle::#lifecycle,
                            initial: #initial, close: #close, close_action: #close_action, single_state: #single_state,
                        })?;
                    #timestamp_check
                });
                quote!(State)
            }
            "reserve" => {
                let binding = required(field, "state")?;
                if !matches!(binding, Expr::Path(path) if path.qself.is_none() && path.path.get_ident() == Some(state_ident))
                {
                    return Err(syn::Error::new_spanned(
                        binding,
                        "reserve state must reference the declared state field",
                    ));
                }
                let values = layout_values(field, &["role", "capacity", "output"])?;
                let (role, capacity, output) = (&values[0], &values[1], &values[2]);
                reserve_layouts.push(quote!(::ckb_contract_core::layout::ReserveLayout {
                    role: #role, capacity: #capacity, output: #output,
                }));
                let asset = required(field, "asset")?;
                let before = required(field, "before")?;
                let after = required(field, "after")?;
                roles.push(role.clone());
                outputs.push(output.clone());
                loads.push(quote! {
                    let #ident: #ty = ::ckb_contract_onchain::patterns::ReservePair::load(#env_ident, &#binding,
                        ::ckb_contract_onchain::patterns::ReservePolicy {
                            role: #role, asset: #asset, capacity: #capacity, output: #output, before: #before, after: #after,
                        })?;
                });
                quote!(Reserve)
            }
            _ => {
                let lock_hash = required(field, "lock_hash")?;
                let protocol = field
                    .args
                    .get("protocol")
                    .map_or_else(|| quote!(#env_ident.base), |e| quote!(#e));
                loads.push(quote! { let #ident: #ty = ::ckb_contract_onchain::context::InputAuthority::load_external(#lock_hash, #protocol)?; });
                quote!(Authority)
            }
        };
        descriptors.push(quote!(::ckb_contract_core::schema::FieldSchema { name: #name, kind: ::ckb_contract_core::schema::FieldKind::#kind, constraints: &[#(#constraints),*] }));
    }
    let guard = no_mint.then(|| quote!(::ckb_contract_onchain::token::reject_standard_mint(&[])?;));
    let check = context_check.as_ref().map(|expr| {
        quote! {
            if !(#expr) { return Err(::ckb_contract_core::Error::Transition.into()); }
        }
    });
    let context_constraints = context_check.into_iter().map(|expr| {
        quote!(::ckb_contract_core::schema::Constraint {
            name: "constraint",
            expression: stringify!(#expr),
        })
    });
    let name = &input.ident;
    let field_names: Vec<_> = fields.iter().map(|f| &f.ident).collect();
    Ok(quote! {
        impl ::ckb_contract_onchain::context::Cells for #name {
            type Error = <#protocol as ::ckb_contract_core::BinaryCodec>::Error;
            type Args<'a> = ::ckb_contract_onchain::patterns::RoleEnvironment<'a>;

            fn load(#env_ident: &Self::Args<'_>) -> ::core::result::Result<Self, Self::Error> {
                #(#layouts)*
                ::ckb_contract_core::layout::validate_layout(&[#(#roles),*], &[#(#outputs),*])?;
                #guard
                #[allow(unused_variables)]
                let args = #env_ident;
                #(#loads)*
                #check
                Ok(Self { #(#field_names),* })
            }

            fn schema() -> ::ckb_contract_core::schema::ContextSchema {
                #(#layouts)*
                ::ckb_contract_core::schema::ContextSchema {
                    version: 1, name: stringify!(#name), reject_standard_mint: #no_mint,
                    single_state: #single_state, state_layout: Some(#state_layout),
                    reserve_layouts: [#(#reserve_layouts),*].into(), fields: &[#(#descriptors),*],
                    constraints: &[#(#context_constraints),*],
                }
            }
        }

    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_source(source: &str) -> syn::Result<Tokens> {
        expand(syn::parse_str(source)?)
    }

    #[test]
    fn generated_code_is_valid_rust_and_preserves_declared_guards_and_expressions() {
        let output = expand_source(r#"
            #[cells(no_mint, single_state)]
            struct Fill {
                #[state(role = STATE, lock_role = LOCK, capacity = CAP, output = 0, mode = "any", initial = initial, close = State::require_close, close_action = Action::Close, update_timestamp = timestamp)]
                order: StateCell<State>,
                #[authority(lock_hash = actor(&order)?)] actor: InputAuthority,
                #[reserve(state = order, role = RESERVE, capacity = CAP, output = 1, asset = order.state().asset, before = remaining(&order)?, after = 0)]
                reserve: ReservePair,
            }
        "#).unwrap();
        let _: syn::File = syn::parse2(output.clone()).unwrap();
        let text = output.to_string();
        assert!(text.contains("reject_standard_mint"));
        assert!(text.contains("validate_layout"));
        assert!(text.contains("actor (& order) ?"));
        assert!(text.contains("State :: require_close"));
        assert!(text.contains("order . require_update_timestamp (timestamp) ?"));
        assert!(text.contains("stringify !"));
    }

    #[test]
    fn shared_layouts_replace_structural_attributes_without_competing_values() {
        let source = r#"
            struct Shared {
                #[state(layout = LAYOUT.state, mode = "update")]
                state: StateCell<State>,
                #[reserve(layout = LAYOUT.reserves[0], state = state, asset = asset, before = 1, after = 1)]
                reserve: ReservePair,
            }
        "#;
        let output = expand_source(source).unwrap();
        let _: syn::File = syn::parse2(output).unwrap();
        for (needle, replacement) in [
            ("layout = LAYOUT.state", "layout = LAYOUT.state, role = 0"),
            (
                "layout = LAYOUT.state",
                "layout = LAYOUT.state, capacity = 400",
            ),
            (
                "layout = LAYOUT.reserves[0]",
                "layout = LAYOUT.reserves[0], output = 1",
            ),
        ] {
            let error = expand_source(&source.replace(needle, replacement)).unwrap_err();
            assert_eq!(
                error.to_string(),
                "layout cannot be combined with role, lock_role, capacity or output"
            );
        }
    }

    #[test]
    fn generic_contexts_require_explicit_selectors_and_valid_field_types() {
        for invalid in [
            "struct Bad { #[cell(source = Source::Input)] value: Cell<Value> }",
            "struct Bad { #[cell(index = 0)] value: Cell<Value> }",
            "struct Bad { #[cell(source = Source::Input, index = 0)] value: u8 }",
            "struct Bad { #[cell(source = Source::Input, index = 0, role = 0)] value: Cell<Value> }",
            "struct Bad { #[cell(source = Source::Input, index = 0)] args: Cell<Value> }",
            "#[cells(single_state)] struct Bad { #[cell(source = Source::Input, index = 0)] value: Cell<Value> }",
            "struct Bad { #[reserve(state = absent)] reserve: ReservePair }",
        ] {
            assert!(expand_source(invalid).is_err(), "accepted {invalid}");
        }
        let output = expand_source(r#"
            #[cells(args = Params<'a>, constraint = before.data().value == after.data().value)]
            struct Generic {
                #[cell(source = Source::Input, type_script = Some(args.script))] before: Cell<Value>,
                #[cell(source = Source::Output, type_script = Some(args.script))] after: Cell<Value>,
            }
        "#).unwrap();
        let _: syn::File = syn::parse2(output).unwrap();
    }

    #[test]
    fn timestamp_policy_requires_an_update_capable_lifecycle() {
        for (mode, extra) in [
            ("create", "initial = initial"),
            ("close", "close = close, close_action = Action::Close"),
        ] {
            let source = format!(
                "struct Bad {{ #[state(role=0, lock_role=1, capacity=1, output=0, \
                 mode=\"{mode}\", {extra}, update_timestamp=timestamp)] state: StateCell<State> }}"
            );
            let error = expand_source(&source).unwrap_err();
            assert_eq!(
                error.to_string(),
                "update_timestamp requires an update lifecycle"
            );
        }
        for mode in ["update", "persist", "any"] {
            let extra = match mode {
                "update" => "",
                "persist" => ", initial=initial",
                _ => ", initial=initial, close=close, close_action=Action::Close",
            };
            let source = format!(
                "struct Good {{ #[state(role=0, lock_role=1, capacity=1, output=0, \
                 mode=\"{mode}\", update_timestamp=timestamp{extra})] state: StateCell<State> }}"
            );
            assert!(expand_source(&source).is_ok());
        }
    }

    #[test]
    fn malformed_or_incomplete_context_policies_are_rejected() {
        for invalid in [
            "#[state(role=0)] struct Bad {}",
            "struct Bad { #[cells(no_mint)] #[state(mode=\"update\")] state: StateCell<State> }",
            "enum Bad { A }",
            "struct Bad(u8);",
            "struct Bad {}",
            "struct Bad<T> { field: T }",
            "struct Bad { field: u8 }",
            "#[cells(unknown)] struct Bad {}",
            "#[cells(no_mint, no_mint)] struct Bad {}",
            "struct Bad { #[state(mode = \"update\")] state: u8 }",
            "struct Bad { #[state(mode = \"invalid\")] state: StateCell<State> }",
            "struct Bad { #[state(mode = \"create\", role=0, lock_role=1, capacity=1, output=0)] state: StateCell<State> }",
            "struct Bad { #[state(mode = \"close\", role=0, lock_role=1, capacity=1, output=0)] state: StateCell<State> }",
            "struct Bad { #[state(mode = \"update\", role=0, role=1)] state: StateCell<State> }",
            "struct Bad { #[state(mode = \"update\", roles=0)] state: StateCell<State> }",
            "struct Bad { #[authority(lock_hash = owner)] actor: InputAuthority, #[state(mode=\"update\")] state: StateCell<State> }",
            "struct Bad { #[state(mode=\"update\", role=0, lock_role=1, capacity=1, output=0)] state: StateCell<State>, #[reserve(state=wrong, role=2)] reserve: ReservePair }",
            "struct Bad { #[state(mode=\"update\", initial=init)] state: StateCell<State> }",
        ] {
            assert!(expand_source(invalid).is_err(), "accepted {invalid}");
        }
    }
}
