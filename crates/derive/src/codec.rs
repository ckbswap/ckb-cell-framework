use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::BTreeSet;
use syn::{Data, DeriveInput, Fields, LitByteStr, LitInt};

fn fields(fields: &Fields) -> Vec<&syn::Field> {
    fields.iter().collect()
}

fn describe(fields: &[&syn::Field]) -> syn::Result<TokenStream> {
    let mut result = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        if let Some(attr) = field
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("codec"))
        {
            return Err(syn::Error::new_spanned(
                attr,
                "codec attributes belong on the type or enum variants",
            ));
        }
        let name = field.ident.as_ref().map_or_else(
            || {
                let index = index.to_string();
                quote!(#index)
            },
            |ident| quote!(stringify!(#ident)),
        );
        let ty = &field.ty;
        result.push(quote!(::ckb_contract_core::wire::WireField {
            name: #name, ty: <#ty as ::ckb_contract_core::wire::WireValue>::TYPE,
        }));
    }
    Ok(quote!(&[#(#result),*]))
}

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "generic CellCodec types are not supported",
        ));
    }
    let name = &input.ident;
    let mut prefix = None;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("codec"))
    {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("prefix") || prefix.is_some() {
                return Err(meta.error("expected one prefix byte string"));
            }
            prefix = Some(meta.value()?.parse::<LitByteStr>()?);
            Ok(())
        })?;
    }
    let prefix = prefix.map_or_else(|| quote!(b""), |prefix| quote!(#prefix));
    let (body, write, read) = match &input.data {
        Data::Struct(data) => {
            if !matches!(data.fields, Fields::Named(_)) {
                return Err(syn::Error::new_spanned(
                    &data.fields,
                    "CellCodec structs require named fields",
                ));
            }
            let fields = fields(&data.fields);
            let description = describe(&fields)?;
            let names: Vec<_> = fields
                .iter()
                .map(|field| field.ident.as_ref().unwrap())
                .collect();
            let types: Vec<_> = fields.iter().map(|field| &field.ty).collect();
            (
                quote!(::ckb_contract_core::wire::WireType::Struct { name: stringify!(#name), fields: #description }),
                quote!(#(::ckb_contract_core::wire::WireValue::write(&self.#names, writer);)*),
                quote!(Ok(Self { #(#names: <#types as ::ckb_contract_core::wire::WireValue>::read(reader)?),* })),
            )
        }
        Data::Enum(data) => {
            if data.variants.is_empty() {
                return Err(syn::Error::new_spanned(
                    name,
                    "CellCodec enums require at least one variant",
                ));
            }
            let mut tags = BTreeSet::new();
            let mut descriptions = Vec::new();
            let mut writes = Vec::new();
            let mut reads = Vec::new();
            for variant in &data.variants {
                if variant.discriminant.is_some() {
                    return Err(syn::Error::new_spanned(
                        variant,
                        "use #[codec(tag = ...)] for wire tags",
                    ));
                }
                let mut tag = None;
                for attr in variant
                    .attrs
                    .iter()
                    .filter(|attr| attr.path().is_ident("codec"))
                {
                    attr.parse_nested_meta(|meta| {
                        if !meta.path.is_ident("tag") || tag.is_some() {
                            return Err(meta.error("expected one u8 tag"));
                        }
                        tag = Some(meta.value()?.parse::<LitInt>()?.base10_parse::<u8>()?);
                        Ok(())
                    })?;
                }
                let tag = tag.ok_or_else(|| {
                    syn::Error::new_spanned(variant, "every variant requires #[codec(tag = ...)]")
                })?;
                if !tags.insert(tag) {
                    return Err(syn::Error::new_spanned(variant, "duplicate wire tag"));
                }
                let ident = &variant.ident;
                let fields = fields(&variant.fields);
                let description = describe(&fields)?;
                let names: Vec<_> = fields
                    .iter()
                    .filter_map(|field| field.ident.as_ref())
                    .collect();
                let types: Vec<_> = fields.iter().map(|field| &field.ty).collect();
                let bindings: Vec<_> = fields
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        format_ident!(
                            "__ckb_codec_field_{index}",
                            span = proc_macro2::Span::mixed_site()
                        )
                    })
                    .collect();
                let pattern = match &variant.fields {
                    Fields::Unit => quote!(Self::#ident),
                    Fields::Named(_) => quote!(Self::#ident { #(#names: #bindings),* }),
                    Fields::Unnamed(_) => quote!(Self::#ident(#(#bindings),*)),
                };
                let construction = match &variant.fields {
                    Fields::Unit => quote!(Self::#ident),
                    Fields::Named(_) => {
                        quote!(Self::#ident { #(#names: <#types as ::ckb_contract_core::wire::WireValue>::read(reader)?),* })
                    }
                    Fields::Unnamed(_) => {
                        quote!(Self::#ident(#(<#types as ::ckb_contract_core::wire::WireValue>::read(reader)?),*))
                    }
                };
                descriptions.push(quote!(::ckb_contract_core::wire::WireVariant { name: stringify!(#ident), tag: #tag, fields: #description }));
                writes.push(quote!(#pattern => { writer.u8(#tag); #(::ckb_contract_core::wire::WireValue::write(#bindings, writer);)* }));
                reads.push(quote!(#tag => Ok(#construction)));
            }
            (
                quote!(::ckb_contract_core::wire::WireType::Enum { name: stringify!(#name), variants: &[#(#descriptions),*] }),
                quote!(match self { #(#writes),* }),
                quote!(match reader.u8()? { #(#reads,)* _ => Err(::ckb_contract_core::Error::Encoding) }),
            )
        }
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "CellCodec requires a struct or enum",
            ))
        }
    };
    Ok(quote! {
        impl ::ckb_contract_core::wire::WireValue for #name {
            const TYPE: ::ckb_contract_core::wire::WireType = #body;
            fn write(&self, writer: &mut ::ckb_contract_core::Writer) { #write }
            fn read(reader: &mut ::ckb_contract_core::Reader<'_>) -> ::ckb_contract_core::Result<Self> { #read }
        }
        impl ::ckb_contract_core::BinaryCodec for #name {
            type Error = ::ckb_contract_core::Error;
            fn encode(&self) -> ::ckb_contract_core::wire::EncodedBytes {
                ::ckb_contract_core::wire::encode(self, #prefix)
            }
            fn decode(data: &[u8]) -> ::ckb_contract_core::Result<Self> {
                ::ckb_contract_core::wire::decode(data, #prefix)
            }
        }
        impl ::ckb_contract_core::wire::CodecSchema for #name {
            const SCHEMA: ::ckb_contract_core::wire::EncodingSchema = ::ckb_contract_core::wire::EncodingSchema {
                prefix: #prefix, body: <Self as ::ckb_contract_core::wire::WireValue>::TYPE,
            };
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ambiguous_tags_unsupported_shapes_and_codec_attributes() {
        for source in [
            "struct Bad(u64);",
            "struct Bad<T> { value: T }",
            "enum Bad {}",
            "enum Bad { Missing }",
            "enum Bad { #[codec(tag=0)] A, #[codec(tag=0)] B }",
            "enum Bad { #[codec(tag=256)] A }",
            "enum Bad { #[codec(tag=0)] A(#[codec(skip)] u8) }",
            "#[codec(unknown=1)] struct Bad { value:u8 }",
            "#[codec(prefix=b\"A\", prefix=b\"B\")] struct Bad { value:u8 }",
            "struct Bad { #[codec(skip)] value:u8 }",
        ] {
            assert!(
                expand(syn::parse_str(source).unwrap()).is_err(),
                "accepted {source}"
            );
        }
    }
}
