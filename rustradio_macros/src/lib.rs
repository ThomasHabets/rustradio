//! Derive macros for rustradio.
//!
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Attribute, Data, DeriveInput, Fields, Meta};

static STRUCT_ATTRS: &[&str] = &["new", "out", "crate"];
static FIELD_ATTRS: &[&str] = &["in", "out"];

// See example at:
// * https://docs.rs/syn/latest/syn/struct.Attribute.html#method.parse_nested_meta
// * https://docs.rs/syn/latest/syn/meta/fn.parser.html
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
            let s = meta.path.get_ident().unwrap();
            if !valid.iter().any(|v| s == v) {
                panic!("Invalid attr {s}");
            }
            found |= meta.path.is_ident(name);
            Ok(())
        })
        .unwrap();
        found
    })
}

/// TODO:
/// * Support all kinds of generic annotations (or none!)
/// * Panic if given invalid attributes.
#[proc_macro_derive(Block, attributes(rustradio))]
pub fn derive_eof(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    //eprintln!("{input:?}");
    let struct_name = input.ident;
    //eprintln!("struct name: {struct_name}");
    let name_str = struct_name.to_string();

    let data_struct = match input.data {
        Data::Struct(d) => d,
        _ => panic!("derive_eof can only be used on structs"),
    };
    let fields_named = match data_struct.fields {
        Fields::Named(f) => f,
        // _ => return quote! { false }.into(),
        x => panic!("Fields is what? {x:?}"),
    };

    let mut set_eofs = vec![];
    let mut eof_checks = vec![];
    let mut extra = vec![];

    // TODO: surely there's a cleaner way.
    let mut ins = vec![];
    let mut insty = vec![];
    let mut outs = vec![];
    let mut outsty = vec![];
    let mut other = vec![];
    let mut otherty = vec![];
    fields_named.named.into_iter().for_each(|field| {
        let field_name = field.ident.clone().unwrap();
        let found_in = has_attr(&field.attrs, "in", FIELD_ATTRS);
        let found_out = has_attr(&field.attrs, "out", FIELD_ATTRS);
        match (found_in, found_out) {
            (true, true) => panic!("Field {field_name} marked both as input and output stream."),
            (false, false) => {
                // panic!("Field {field:?} marked neither input nor output");
                let ty = field.ty;
                other.push(field_name.clone());
                otherty.push(quote! { #field_name: #ty});
            }
            (false, true) => {
                set_eofs.push(quote! { self.#field_name.set_eof(); });
                outs.push(field_name.clone());
                let ty = field.ty;
                outsty.push(quote! { #ty });
            }
            (true, false) => {
                eof_checks.push(quote! { self.#field_name.eof() });
                ins.push(field_name.clone());
                let ty = field.ty;
                insty.push(quote! { #field_name: #ty });
            }
        };
    });

    if has_attr(&input.attrs, "new", STRUCT_ATTRS) {
        extra.push(quote! {
            impl<T: Copy> #struct_name<T> {
                pub fn new(#(#insty),*,#(#otherty),*) -> Self {
                    Self {
                    #(#ins),*,
                    #(#outs: Stream::newp()),*,
                    #(#other: #other),*
                    }
                }
            }
        });
    }
    if has_attr(&input.attrs, "out", STRUCT_ATTRS) {
        extra.push(quote! {
            impl<T: Copy> #struct_name<T> {
                pub fn out(&self) -> (#(#outsty),*) {
                    (#(self.#outs.clone()),*)
                }
            }
        });
    }
    let path = match has_attr(&input.attrs, "crate", STRUCT_ATTRS) {
        true => quote! { crate::block },
        false => quote! { rustradio::block },
    };

    let expanded = quote! {
        impl<T: Copy> #path::BlockName for #struct_name<T> {
            fn block_name(&self) -> &str {
                #name_str
            }
        }
        impl<T: Copy> #path::BlockEOF for #struct_name<T> {
            fn eof(&mut self) -> bool {
                if true #(&&#eof_checks)* {
                    #(#set_eofs)*
                    true
                } else {
                    false
                }
            }
        }
        #(#extra)*
    };
    TokenStream::from(expanded)
}
