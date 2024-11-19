//! Derive macros for rustradio.
//!
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Attribute, Data, DeriveInput, Fields, Meta};

fn has_attr<'a, I: IntoIterator<Item = &'a Attribute>>(attrs: I, name: &str) -> bool {
    attrs.into_iter().any(|attr| {
        let meta_list = match &attr.meta {
            Meta::List(meta_list) => meta_list,
            _ => return false,
        };
        //eprintln!("  {:?}", attr.meta);
        if !meta_list.path.is_ident("rustradio") {
            return false;
        }
        //eprintln!(" -> {:?}", meta_list.tokens);
        for meta in meta_list.tokens.clone().into_iter() {
            //eprintln!("meta: {:?}", meta);
            match meta {
                proc_macro2::TokenTree::Ident(ident) => {
                    //eprintln!("ident! {ident:?}"),
                    if ident.to_string() == name {
                        return true;
                    }
                }
                m => panic!("Unknown meta {m:?}"),
            }
        }
        false
    })
}

/// TODO:
/// * Support non-stream member variables in generated new()
/// * Support all kings of type annotations (or none!)
/// * Panic if given invalid attributes.
#[proc_macro_derive(Block, attributes(rustradio))]
pub fn derive_eof(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

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

    let mut ins = vec![];
    let mut outs = vec![];
    fields_named.named.into_iter().for_each(|field| {
        let field_name = field.ident.clone().unwrap();
        let found_in = has_attr(&field.attrs, "in");
        let found_out = has_attr(&field.attrs, "out");
        match (found_in, found_out) {
            (true, true) => panic!("Field {field_name} marked both as input and output stream."),
            (false, false) => {
                // panic!("Field {field:?} marked neither input nor output");
            }
            (false, true) => {
                set_eofs.push(quote! { self.#field_name.set_eof(); });
                outs.push(field_name.clone());
            }
            (true, false) => {
                eof_checks.push(quote! { self.#field_name.eof() });
                ins.push(field_name);
            }
        };
    });

    if has_attr(&input.attrs, "new") {
        extra.push(quote! {
            impl<T> #struct_name<T>
            where
                T: Copy,
            {
                pub fn new(#(#ins: Streamp<T>),*) -> Self {
                    Self {
                    #(#ins),*,
                    #(#outs: Stream::newp()),* }
                }
            }
        });
    }
    if has_attr(&input.attrs, "new") {
        let rval = (0..outs.len()).map(|_| quote! { Streamp<T> });
        extra.push(quote! {
            impl<T> #struct_name<T>
            where
                T: Copy,
            {
                pub fn out(&self) -> (#(#rval),*) {
                    (#(self.#outs.clone()),*)
                }
            }
        });
    }

    let expanded = quote! {
        impl<T> AutoBlock for #struct_name<T>
        where
            T: Copy,
        {
            fn name(&self) -> &str {
                #name_str
            }
            fn eof(&mut self) -> bool {
                if #(#eof_checks)&&* {
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
