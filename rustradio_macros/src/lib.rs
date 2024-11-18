use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Data, Fields, Meta};

#[proc_macro_derive(Eof, attributes(rustradio))]
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

    let checks = fields_named.named.into_iter().filter_map(|field| {
        let field_name = field.ident?;
        //eprintln!("{field_name}");
        let has_in_attr = field.attrs.iter().any(|attr| {
            let meta_list = match &attr.meta {
                Meta::List(meta_list) => meta_list,
                _ => return false,
            };
            //eprintln!("  {:?}", attr.meta);
            if !meta_list.path.is_ident("rustradio") {
                return false;
            }
            //eprintln!(" -> {:?}", meta_list.tokens);
            let mut found_in = false;
            let mut found_out = false;
            for meta in meta_list.tokens.clone().into_iter() {
                //eprintln!("meta: {:?}", meta);
                match meta {
                    proc_macro2::TokenTree::Ident(ident) => {
                        //eprintln!("ident! {ident:?}"),
                        match ident.to_string().as_str() {
                            "in" => found_in = true,
                            "out" => found_out = true,
                            x => panic!("unknown attribute {x}"),
                        }
                    },
                    m => panic!("Unknown meta {m:?}"),
                }
            }
            if found_out && found_in {
                panic!("Field {field_name} marked both as input and output stream.");
            }
            if found_out {
                set_eofs.push(quote! {
                    self.#field_name.set_eof();
                })
            }
            // TODO: if type stream and not in nor out, panic.
            found_in
        });
        //eprintln!("has in attr? {has_in_attr}");
        if has_in_attr {
            Some(quote! {
                self.#field_name.eof()
            })
        } else {
            None
        }
    });
    let fields_check = quote! {
        true #(&& #checks)*
    };
    //eprintln!("check: {fields_check:?}");

    // Generate the implementation of the `eof` function
    let expanded = quote! {
        impl<T> AutoBlock for #struct_name<T>
        where
            T: Copy,
        {
            fn name(&self) -> &str {
                #name_str
            }
            fn eof(&mut self) -> bool {
                if #fields_check {
                    #(#set_eofs)*
                    true
                } else {
                    false
                }
            }
        }
    };

    TokenStream::from(expanded)
}

