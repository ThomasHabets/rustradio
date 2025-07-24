use std::borrow::Cow;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, Meta};

static FIELD_ATTRS: &[&str] = &["in", "out", "default", "into"];

/// Check if named attribute is in the list of attributes.
///
/// Panic if there's an attribute not in the valid list provided.
// See example at:
// * https://docs.rs/syn/latest/syn/struct.Attribute.html#method.parse_nested_meta
// * https://docs.rs/syn/latest/syn/meta/fn.parser.html
#[must_use]
fn has_attr<'a, I: IntoIterator<Item = &'a Attribute>>(
    attrs: I,
    name: &str,
    valid: &[&str],
) -> bool {
    attrs.into_iter().any(|attr| {
        //eprintln!("{:?}", attr);
        let meta_list = match &attr.meta {
            Meta::List(meta_list) => meta_list,
            _ => return false,
        };
        //eprintln!("  {:?}", attr.meta);
        if !meta_list.path.is_ident("rustradio") {
            return false;
        }
        let mut found = false;
        attr.parse_nested_meta(|meta| {
            let s = meta.path.get_ident().expect("path without ident");
            if !valid.iter().any(|v| s == v) {
                panic!("Invalid attr {s}");
            }
            found |= meta.path.is_ident(name);
            Ok(())
        })
        .expect("parse_nested_meta()");
        found
    })
}

/// Return the inner type of a generic type.
///
/// E.g. given ReadStream<Float>, return Float.
#[must_use]
fn inner_type(ty: &syn::Type) -> &syn::Type {
    if let syn::Type::Path(p) = &ty {
        let segment = p.path.segments.last().unwrap();
        //assert_eq!(segment.ident, "Streamp");
        if let syn::PathArguments::AngleBracketed(angle_bracketed_args) = &segment.arguments {
            for arg in &angle_bracketed_args.args {
                if let syn::GenericArgument::Type(ty) = arg {
                    return ty;
                }
            }
        }
    }
    panic!(
        "Tried to get the inner type of a non-generic, probably non-Stream: {}",
        quote! { #ty }
    )
}

#[derive(Default, PartialEq, Debug)]
enum Sync {
    // `sync`. Pass tags through as is.
    Value,
    // `sync_tag`. Also allow tag modification.
    Tag,
    // `sync_nocopy_tag`. Like `sync_tag` but for NoCopy.
    NoCopyTag,
    #[default]
    General,
}

#[derive(Default)]
struct StructAttrs {
    internal: bool,
    custom_name: bool,
    generate_new: bool,
    noeof: bool,
    sync: Sync,
    bounds: Option<syn::WhereClause>,
}

impl StructAttrs {
    #[must_use]
    fn path(&self) -> proc_macro2::TokenStream {
        if self.internal {
            quote! { crate }
        } else {
            quote! { rustradio }
        }
    }
    #[must_use]
    fn parse(attrs: &[Attribute]) -> StructAttrs {
        let mut ret = StructAttrs::default();
        let mut bounds = Vec::new();
        attrs
            .iter()
            .filter_map(|attr| match &attr.meta {
                Meta::List(l) => Some(l),
                _ => None,
            })
            .filter(|list| list.path.is_ident("rustradio"))
            .for_each(|list| {
                list.parse_nested_meta(|meta| {
                    let s = meta.path.get_ident().expect("failed to get ident");
                    match s.to_string().as_str() {
                        "bound" => {
                            let value = meta.value()?;
                            let lit: syn::LitStr = value.parse()?;
                            bounds.push(lit.value());
                        }
                        "crate" => ret.internal = true,
                        "custom_name" => ret.custom_name = true,
                        "noeof" => ret.noeof = true,
                        "new" => ret.generate_new = true,
                        "sync" => {
                            assert_eq!(ret.sync, Sync::General, "Only one sync tag can be used");
                            ret.sync = Sync::Value
                        }
                        "sync_tag" => {
                            assert_eq!(ret.sync, Sync::General, "Only one sync tag can be used");
                            ret.sync = Sync::Tag
                        }
                        "sync_nocopy_tag" => {
                            assert_eq!(ret.sync, Sync::General, "Only one sync tag can be used");
                            ret.sync = Sync::NoCopyTag
                        }
                        other => panic!("invalid attr {other}"),
                    }
                    Ok(())
                })
                .unwrap();
            });
        let w: syn::WhereClause = syn::parse_str(&format!("where {}", bounds.join(","))).unwrap();
        ret.bounds = Some(w);
        ret
    }
}

#[must_use]
fn merge_where_clauses(
    struct_clause: Option<&syn::WhereClause>,
    macro_clause: Option<&syn::WhereClause>,
) -> Option<syn::WhereClause> {
    match (struct_clause, macro_clause) {
        (None, None) => None,
        (Some(clause), None) | (None, Some(clause)) => Some(clause.clone()),
        (Some(struct_clause), Some(macro_clause)) => {
            let mut combined = struct_clause.clone();
            combined.predicates.extend(macro_clause.predicates.clone());
            Some(combined)
        }
    }
}

struct Parsed<'a> {
    name: &'a syn::Ident,
    attrs: StructAttrs,
    generics: (
        syn::ImplGenerics<'a>,
        syn::TypeGenerics<'a>,
        Option<syn::WhereClause>,
    ),
    struct_where: Option<&'a syn::WhereClause>,
    inputs: Vec<&'a syn::Field>,
    outputs: Vec<&'a syn::Field>,
    defaults: Vec<&'a syn::Field>,
    parms: Vec<(bool, &'a syn::Field)>,
}

impl<'a> Parsed<'a> {
    fn parse(input: &'a DeriveInput) -> Result<Self, std::fmt::Error> {
        let Data::Struct(data_struct) = &input.data else {
            panic!("can only use on struct");
        };
        let Fields::Named(fields_named) = &data_struct.fields else {
            panic!("Fields is what? {:?}", data_struct.fields);
        };
        let attrs = StructAttrs::parse(&input.attrs);
        let (generics, struct_where) = {
            let (a, b, w) = input.generics.split_for_impl();
            let w2 = merge_where_clauses(w, attrs.bounds.as_ref());
            ((a, b, w2), w)
        };
        Ok(Self {
            name: &input.ident,
            attrs,
            generics,
            struct_where,
            inputs: fields_named
                .named
                .iter()
                .filter(|field| has_attr(&field.attrs, "in", FIELD_ATTRS))
                .collect(),
            outputs: fields_named
                .named
                .iter()
                .filter(|field| has_attr(&field.attrs, "out", FIELD_ATTRS))
                .collect(),
            defaults: fields_named
                .named
                .iter()
                .filter(|field| has_attr(&field.attrs, "default", FIELD_ATTRS))
                .collect(),
            parms: fields_named
                .named
                .iter()
                .filter(|field| {
                    !has_attr(&field.attrs, "in", FIELD_ATTRS)
                        && !has_attr(&field.attrs, "out", FIELD_ATTRS)
                        && !has_attr(&field.attrs, "default", FIELD_ATTRS)
                })
                .map(|field| (has_attr(&field.attrs, "into", FIELD_ATTRS), field))
                .collect(),
        })
    }
    #[must_use]
    fn in_name_types(&self) -> Vec<proc_macro2::TokenStream> {
        self.inputs
            .iter()
            .map(|field| {
                let n = &field.ident;
                let ty = &field.ty;
                quote! { #n: #ty }
            })
            .collect()
    }
    #[must_use]
    fn parm_name_types(&self) -> Vec<proc_macro2::TokenStream> {
        self.parms
            .iter()
            .map(|(is_into, field)| {
                let name = field.ident.as_ref().unwrap();
                let ty = if *is_into {
                    Cow::Owned(syn::parse_str(&format!("Into{name}")).unwrap())
                } else {
                    Cow::Borrowed(&field.ty)
                };
                quote! { #name: #ty }
            })
            .collect()
    }
    #[must_use]
    fn in_names(&self) -> Vec<&syn::Ident> {
        self.inputs
            .iter()
            .map(|field| field.ident.as_ref().unwrap())
            .collect()
    }
    #[must_use]
    fn out_names(&self) -> Vec<&syn::Ident> {
        self.outputs
            .iter()
            .map(|field| field.ident.as_ref().unwrap())
            .collect()
    }
    #[must_use]
    fn in_tag_names(&self) -> Vec<syn::Ident> {
        self.inputs
            .iter()
            .map(|field| {
                let name = field.ident.as_ref().unwrap();
                syn::parse_str(&format!("{name}_tag")).unwrap()
            })
            .collect()
    }
    #[must_use]
    fn out_tag_names(&self) -> Vec<syn::Ident> {
        self.outputs
            .iter()
            .map(|field| {
                let name = field.ident.as_ref().unwrap();
                syn::parse_str(&format!("{name}_tag")).unwrap()
            })
            .collect()
    }
    #[must_use]
    fn out_tag_names_tmp(&self) -> Vec<syn::Ident> {
        self.outputs
            .iter()
            .map(|field| {
                let name = field.ident.as_ref().unwrap();
                syn::parse_str(&format!("{name}_tag_tmp")).unwrap()
            })
            .collect()
    }
    #[must_use]
    fn out_names_samp(&self) -> Vec<syn::Ident> {
        self.outputs
            .iter()
            .map(|field| {
                let name = field.ident.as_ref().unwrap();
                syn::parse_str(&format!("{name}_sample")).unwrap()
            })
            .collect()
    }
    #[must_use]
    fn parm_into_names(&self) -> Vec<&syn::Ident> {
        self.parms
            .iter()
            .filter_map(|(is_into, field)| {
                if *is_into {
                    Some(field.ident.as_ref().unwrap())
                } else {
                    None
                }
            })
            .collect()
    }
    #[must_use]
    fn parm_no_into_names(&self) -> Vec<&syn::Ident> {
        self.parms
            .iter()
            .filter_map(|(is_into, field)| {
                if *is_into {
                    None
                } else {
                    Some(field.ident.as_ref().unwrap())
                }
            })
            .collect()
    }
    #[must_use]
    fn parm_into_types(&self) -> Vec<TokenStream> {
        self.parms
            .iter()
            .filter_map(|(is_into, field)| {
                if *is_into {
                    let ty = &field.ty;
                    let field_name = field.ident.as_ref().unwrap();
                    let gen_name: syn::Type = syn::parse_str(&format!("Into{field_name}")).unwrap();
                    Some(quote! { #gen_name: Into<#ty> })
                } else {
                    None
                }
            })
            .collect()
    }
    #[must_use]
    fn fields_defaulted(&self) -> Vec<TokenStream> {
        self.defaults
            .iter()
            .map(|field| {
                let field_name = field.ident.as_ref().unwrap();
                let ty = &field.ty;
                quote! { #field_name: <#ty>::default() }
            })
            .collect()
    }
    #[must_use]
    fn outval_types(&self) -> Vec<&syn::Type> {
        self.outputs
            .iter()
            .map(|field| inner_type(&field.ty))
            .collect()
    }
    #[must_use]
    fn inval_name_types(&self) -> Vec<TokenStream> {
        self.inputs
            .iter()
            .map(|field| {
                let inner = inner_type(&field.ty);
                let name = field.ident.as_ref().unwrap();
                quote! { #name: #inner }
            })
            .collect()
    }
    #[must_use]
    fn intag_name_types(&self) -> Vec<TokenStream> {
        let path = self.attrs.path();
        self.inputs
            .iter()
            .map(|field| {
                let name = field.ident.as_ref().unwrap();
                let tagname: syn::Ident = syn::parse_str(&format!("{name}_tag")).unwrap();
                quote! { #tagname: &'a [#path::stream::Tag] }
            })
            .collect()
    }

    #[must_use]
    fn expand_sync_nocopy_work(&self) -> Option<TokenStream> {
        match self.attrs.sync {
            Sync::NoCopyTag => {}
            Sync::General | Sync::Tag | Sync::Value => return None,
        }
        let name = self.name;
        let path = self.attrs.path();
        let (impl_generics, ty_generics, where_clause) = &self.generics;
        let in_names = self.in_names();
        let in_tag_names = self.in_tag_names();
        let out_names = self.out_names();
        let out_tag_names = self.out_tag_names();
        Some(quote! {
            impl #impl_generics #path::block::Block for #name #ty_generics #where_clause {
                fn work(&mut self) -> #path::Result<#path::block::BlockRet> {
                    use #path::block::BlockRet;
                    loop {
                        #(if self.#out_names.remaining() == 0 {
                            return Ok(BlockRet::WaitForStream(&self.#out_names, 1));
                        })*
                        #(if self.#in_names.is_empty() {
                            return Ok(BlockRet::WaitForStream(&self.#in_names, 1));
                        })*
                        #(let (#in_names, #in_tag_names) = self.#in_names.pop().expect("can't happen: we checked");)*
                        let (#(#out_names, #out_tag_names),*) = self.process_sync_tags(#(#in_names, &#in_tag_names),*);
                        #(self.#out_names.push(#out_names, #out_tag_names);)*
                    }
                }
            }
        })
    }

    #[must_use]
    fn expand_sync_work(&self) -> Option<TokenStream> {
        match self.attrs.sync {
            Sync::General | Sync::NoCopyTag => return None,
            Sync::Tag | Sync::Value => {}
        }
        let name = self.name;
        let path = self.attrs.path();
        let (impl_generics, ty_generics, where_clause) = &self.generics;
        let in_names = self.in_names();
        let out_names = self.out_names();
        let out_tag_names = self.out_tag_names();
        let out_tag_names_tmp = self.out_tag_names_tmp();
        let out_names_samp = self.out_names_samp();
        let in_tag_names = self.in_tag_names();
        let first = &in_names[0];
        let rest = &in_names[1..];
        let it = if in_names.len() == 1 {
            quote! { #first.iter().take(n) }
        } else {
            quote! { #first.iter().take(n)#(.zip(#rest.iter()))* }
        };
        Some(quote! {
            impl #impl_generics #path::block::Block for #name #ty_generics #where_clause {
                fn work(&mut self) -> #path::Result<#path::block::BlockRet> {
                    let empty = vec![];
                    loop {
                        #(let #in_names = self.#in_names.read_buf()?;)*
                        #(let #in_tag_names = #in_names.1;)*
                        #(let #in_names = #in_names.0;
                          if #in_names.len() == 0 {
                              return Ok(#path::block::BlockRet::WaitForStream(&self.#in_names, 1));
                          })*

                        // Clamp n to be no more than the input available.
                        let n = [#(#in_names.len()),*].iter().fold(usize::MAX, |min, &x|min.min(x));
                        assert_ne!(n, 0, "Input stream len 0, but we already checked that.");

                        #(let mut #out_names = self.#out_names.write_buf()?;
                          if #out_names.len() == 0 {
                              return Ok(#path::block::BlockRet::WaitForStream(&self.#out_names, 1));
                          })*

                        // Clamp n to be no more than output space.
                        let n = [#(#out_names.len()),*].iter().fold(n, |min, &x|min.min(x));
                        assert_ne!(n, 0, "Output stream len 0, but we already checked that.");

                        #(let mut #out_tag_names = Vec::new();)*
                        let empty_tags = true #(&&#in_tag_names.is_empty())*;
                        let it = #it.enumerate().map(|(pos, (#(#in_names),*))| {
                            let (#(#in_tag_names),*) = if empty_tags {
                                // Fast path for input without tags.
                                // There may be opportunity to deduplicate some of
                                // the next couple of lines with the !empty_tags
                                // case.
                                (#({
                                    let _ = &#in_tag_names;
                                    std::borrow::Cow::Borrowed(&empty)
                                }),*)
                            } else {
                                // TODO: This tag filtering is quite expensive.
                                (#(std::borrow::Cow::Owned(#in_tag_names.iter()
                                  .filter(|t| t.pos() == pos)
                                  .map(|t| #path::stream::Tag::new(0, t.key().to_string(), t.val().clone()))
                                  .collect::<Vec<_>>())),*)
                            };
                            let (#(#out_names, #out_tag_names_tmp),*) = self.process_sync_tags(#(*#in_names, &#in_tag_names),*);
                            #(for tag in #out_tag_names_tmp.iter() {
                                #out_tag_names.push(#path::stream::Tag::new(pos, tag.key(), tag.val().clone()));
                            })*
                            (#(#out_names),*)
                        });
                        for ((#(#out_names_samp),*), #(#out_names,)*) in itertools::izip!(it, #(#out_names.slice().iter_mut()),*) {
                            (#(*#out_names),*) = (#(#out_names_samp),*);
                        }
                        #(#in_names.consume(n);)*
                        #(#out_names.produce(n, &#out_tag_names);)*
                    }
                }
            }
        })
    }
    #[must_use]
    fn expand_sync_tags(&self) -> Option<TokenStream> {
        if !matches![self.attrs.sync, Sync::Value] {
            return None;
        }
        let name = self.name;
        let path = self.attrs.path();
        let (impl_generics, ty_generics, where_clause) = &self.generics;
        let inval_name_types = self.inval_name_types();
        let intag_name_types = self.intag_name_types();
        let outval_types = self.outval_types();
        let in_names = self.in_names();
        let out_names = self.out_names();
        let in_tag_names = self.in_tag_names();
        let first_tags = &in_tag_names[0];
        Some(quote! {
                impl #impl_generics #name #ty_generics #where_clause {
                    #[must_use]
                    fn process_sync_tags<'a>(&mut self, #(#inval_name_types, #intag_name_types,)*) -> (#(#outval_types, std::borrow::Cow<'a, [#path::stream::Tag]>),*) {
                        let (#(#out_names),*) = self.process_sync(#(#in_names,)*);
                        (#(#out_names,std::borrow::Cow::Borrowed(#first_tags)),*)
                    }
                }
        })
    }
    #[must_use]
    fn expand_new(&self) -> Option<TokenStream> {
        if !self.attrs.generate_new {
            return None;
        }
        let name = self.name;
        let (impl_generics, ty_generics, where_clause) = &self.generics;
        let in_names = self.in_names();
        let in_name_types = self.in_name_types();
        let out_names = self.out_names();
        let parm_into_types = self.parm_into_types();
        let parm_into_names = self.parm_into_names();
        let parm_no_into_names = self.parm_no_into_names();
        let parm_name_types = self.parm_name_types();
        let fields_defaulted = self.fields_defaulted();
        let path = self.attrs.path();
        let out_types: Vec<_> = self.outputs.iter().map(|field| &field.ty).collect();
        Some(quote! {
            impl #impl_generics #name #ty_generics #where_clause {
                #[must_use]
                pub fn new #(<#parm_into_types>),*(#(#in_name_types,)*#(#parm_name_types),*) -> (Self #(,<#out_types as #path::stream::StreamReadSide>::ReadSide)*) {
                    #(let #out_names = <#out_types>::new();)*
                    (Self {
                        #(#in_names,)*
                        #(#out_names: #out_names.0,)*
                        #(#parm_into_names: #parm_into_names.into(),)*
                        #(#parm_no_into_names,)*
                        #(#fields_defaulted,)*
                    }#(,#out_names.1)*)
                }
            }
        })
    }
    #[must_use]
    fn expand_blockname(&self) -> Option<TokenStream> {
        let name = self.name;
        let name_str = name.to_string();
        let nameval = if self.attrs.custom_name {
            quote! { self.custom_name() }
        } else {
            quote! { #name_str }
        };
        let (impl_generics, ty_generics, _) = &self.generics;
        let struct_where = &self.struct_where;
        let path = self.attrs.path();
        Some(quote! {
            impl #impl_generics #path::block::BlockName for #name #ty_generics #struct_where {
            fn block_name(&self) -> &str {
                #nameval
            }
        }
        })
    }
    #[must_use]
    fn expand_eof(&self) -> Option<TokenStream> {
        if self.attrs.noeof {
            return None;
        }
        let name = self.name;
        let path = self.attrs.path();
        let (impl_generics, ty_generics, _) = &self.generics;
        let where_clause = &self.struct_where;
        let in_names = self.in_names();
        if in_names.is_empty() {
            // TODO: should we really generate this eof() just because there are no
            // inputs?
            return Some(quote! {
                 impl #impl_generics #path::block::BlockEOF for #name #ty_generics #where_clause {
                    fn eof(&mut self) -> bool {
                        false
                    }
                 }
            });
        }
        Some(quote! {
             impl #impl_generics #path::block::BlockEOF for #name #ty_generics #where_clause {
                fn eof(&mut self) -> bool {
                    #(self.#in_names.eof())&&*
                }
             }
        })
    }
    #[must_use]
    fn expand(&self) -> TokenStream {
        let e: Vec<_> = [
            self.expand_new(),
            self.expand_sync_nocopy_work(),
            self.expand_sync_work(),
            self.expand_sync_tags(),
            self.expand_blockname(),
            self.expand_eof(),
        ]
        .into_iter()
        .flatten()
        .collect();
        quote! {
            #(#e)*
        }
    }
}

/// Backend function for the rustradio_macros::Block derive macro.
///
/// Use the macro, not this function.
#[must_use]
pub fn derive_block(input: TokenStream) -> TokenStream {
    let input = syn::parse2::<syn::DeriveInput>(input).unwrap();
    let p = Parsed::parse(&input).unwrap();
    // Sanity check.
    match p.attrs.sync {
        Sync::Value | Sync::Tag | Sync::NoCopyTag => {
            assert!(!p.inputs.is_empty());
            assert!(!p.outputs.is_empty());
        }
        Sync::General => {}
    }
    p.expand()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_baseline() {
        let input = quote! { struct MyBlock {} };
        let actual = derive_block(input);

        assert!(actual.to_string().contains("BlockEOF for MyBlock"));
        assert!(actual.to_string().contains("BlockName for MyBlock"));
        assert!(!actual.to_string().contains("process_sync"));
        assert!(!actual.to_string().contains("custom_name"));
        assert!(
            !actual.to_string().contains("fn work "),
            "{}",
            actual.to_string()
        );
        assert!(
            !actual.to_string().contains("fn new "),
            "{}",
            actual.to_string()
        );
        assert!(
            !actual.to_string().contains("fn out "),
            "{}",
            actual.to_string()
        );
        assert!(actual.to_string().contains("fn eof "));
    }

    #[test]
    fn derive_minimal() {
        let input = quote! { struct MyBlock {} };
        let actual = derive_block(input);
        let expected = quote! {
            impl rustradio::block::BlockName for MyBlock {
                fn block_name(&self) -> &str {
                    "MyBlock"
                }
            }
            impl rustradio::block::BlockEOF for MyBlock {
                fn eof (& mut self) -> bool { false }
            }
        };
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn derive_some_options() {
        let input = quote! {
            #[rustradio(crate, new, custom_name)]
            struct MyBlock {
                #[rustradio(in)]
                src: ReadStream<Float>,
                #[rustradio(out)]
                dst: WriteStream<Complex>,
                #[rustradio(default)]
                foo: u32,
                bar: Float,
                #[rustradio(into)]
                baz: usize,
            }
        };
        let actual = derive_block(input);
        let expected = quote! {
            impl MyBlock {
                #[must_use]
                pub fn new<Intobaz: Into<usize> >(
                    src: ReadStream <Float>,
                    bar: Float,
                    baz: Intobaz) -> (Self, <WriteStream<Complex> as crate::stream::StreamReadSide>::ReadSide) {
                    let dst = <WriteStream<Complex> >::new();
                    (Self {
                        src,
                        dst: dst.0,
                        baz: baz.into(),
                        bar,
                        foo: <u32>::default(),
                    }, dst.1)
                }
            }
            impl crate::block::BlockName for MyBlock {
                fn block_name(&self) -> &str {
                    self.custom_name()
                }
            }
            impl crate::block::BlockEOF for MyBlock {
                fn eof (& mut self) -> bool { self.src.eof() }
            }
        };
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn derive_custom_name() {
        let input = quote! { #[rustradio(custom_name)] struct MyBlock {} };
        let actual = derive_block(input);
        assert!(actual.to_string().contains("custom_name"));
    }

    #[test]
    fn derive_noeof() {
        let input = quote! { #[rustradio(noeof)] struct MyBlock {} };
        let actual = derive_block(input);
        assert!(
            !actual.to_string().contains("fn eof "),
            "{}",
            actual.to_string()
        );
    }

    #[test]
    #[should_panic]
    fn derive_sync_no_input() {
        let _ = derive_block(quote! {
            #[rustradio(sync)]
            struct MyBlock {
                #[rustradio(out)]
                dst: WriteStream<Float>,
            }
        });
    }

    #[test]
    #[should_panic]
    fn derive_sync_tag_no_output() {
        let _ = derive_block(quote! {
            #[rustradio(sync)]
            struct MyBlock {
                #[rustradio(in)]
                src: ReadStream<Float>,
            }
        });
    }

    #[test]
    #[should_panic]
    fn derive_sync_tag_no_input() {
        let _ = derive_block(quote! {
            #[rustradio(sync)]
            struct MyBlock {
                #[rustradio(out)]
                dst: WriteStream<Float>,
            }
        });
    }

    #[test]
    #[should_panic]
    fn derive_sync_no_output() {
        let _ = derive_block(quote! {
            #[rustradio(sync)]
            struct MyBlock {
                #[rustradio(in)]
                src: ReadStream<Float>,
            }
        });
    }

    #[test]
    fn derive_sync() {
        let input = quote! {
            #[rustradio(sync)]
            struct MyBlock {
                #[rustradio(in)]
                src: ReadStream<Float>,
                #[rustradio(out)]
                dst: WriteStream<Float>,
            }
        };
        let actual = derive_block(input);
        assert!(
            actual.to_string().contains("fn work "),
            "{}",
            actual.to_string()
        );
        assert!(!actual.to_string().contains("fn process_sync "));
        assert!(
            actual.to_string().contains("fn process_sync_tags "),
            "{}",
            actual.to_string()
        );
        assert!(actual.to_string().contains("process_sync_tags "));
    }

    #[test]
    fn derive_sync_tag() {
        let input = quote! {
            #[rustradio(sync_tag)]
            struct MyBlock {
                #[rustradio(in)]
                src: ReadStream<Float>,
                #[rustradio(out)]
                dst: WriteStream<Float>,
            }
        };
        let actual = derive_block(input);
        assert!(
            actual.to_string().contains("fn work "),
            "{}",
            actual.to_string()
        );
        assert!(
            !actual.to_string().contains("fn process_sync"),
            "{}",
            actual.to_string()
        );
    }

    #[test]
    fn derive_struct_bad_combo() {
        for (name, q) in [
            (
                "sync and sync_tag",
                quote! {#[rustradio(sync,sync_tag)] struct B { } },
            ),
            (
                "sync and sync_nocopy_tag",
                quote! {#[rustradio(sync,sync_nocopy_tag)] struct B { } },
            ),
            (
                "sync_tag and sync_nocopy_tag",
                quote! {#[rustradio(sync_tag,sync_nocopy_tag)] struct B { } },
            ),
            ("empty bound", quote! {#[rustradio(bound)] struct B { } }),
            ("unknown attr", quote! {#[rustradio(in)] struct B { } }),
        ]
        .into_iter()
        {
            let result = std::panic::catch_unwind(|| {
                let _ = derive_block(q);
            });
            assert!(result.is_err(), "Expected {name} to panic. It didn't");
        }
    }

    #[test]
    fn derive_field_bad_combo() {
        for (name, q) in [
            // In and.
            ("in and out", quote! {struct B { #[rustradio(in, out)] } }),
            ("in and into", quote! {struct B { #[rustradio(in, into)] } }),
            (
                "in and default",
                quote! {struct B { #[rustradio(in, default)] } },
            ),
            // Out and.
            (
                "out and default",
                quote! {struct B { #[rustradio(default, out)] } },
            ),
            (
                "out and into",
                quote! {struct B { #[rustradio(out, into)] } },
            ),
            // Default and.
            (
                "into and default",
                quote! {struct B { #[rustradio(default, into)] } },
            ),
            // Unknown.
            ("unknown arg", quote! {struct B { #[rustradio(new)] } }),
        ]
        .into_iter()
        {
            let result = std::panic::catch_unwind(|| {
                let _ = derive_block(q);
            });
            assert!(result.is_err(), "Expected {name} to panic. It didn't");
        }
    }
}
