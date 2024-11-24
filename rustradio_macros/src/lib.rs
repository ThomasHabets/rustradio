//! Derive macros for rustradio.
//!
//! Most blocks should derive from this macro.
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Attribute, Data, DeriveInput, Fields, Meta};

static STRUCT_ATTRS: &[&str] = &[
    "new",
    "out",
    "crate",
    "sync",
    "sync_tag",
    "custom_name",
    "noeof",
    "nevereof",
];
static FIELD_ATTRS: &[&str] = &["in", "out", "default"];

/// Check if named attribute is in the list of attributes.
///
/// Panic if there's an attribute not in the valid list provided.
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

/// Return the inner type of a generic type.
///
/// E.g. given Streamp<Float>, return Float.
fn inner_type(ty: &syn::Type) -> &syn::Type {
    if let syn::Type::Path(p) = &ty {
        let segment = p.path.segments.last().unwrap();
        // assert_eq!(segment.ident, "Streamp");
        if let syn::PathArguments::AngleBracketed(angle_bracketed_args) = &segment.arguments {
            for arg in &angle_bracketed_args.args {
                if let syn::GenericArgument::Type(ty) = arg {
                    return ty;
                }
            }
        }
    }
    panic!(
        "Tried to get the inner type of a non-generic, probably non-Streamp: {}",
        quote! { #ty }.to_string()
    )
}

/// Block derive macro.
///
/// Most blocks should derive from `Block`. Example use:
///
/// ```
/// #[derive(rustradio_macros::Block)]
/// #[rustradio(new, out)]
/// pub struct MyBlock<T: Copy> {
///   #[rustradio(in)]
///   src: Streamp<T>,
///   #[rustradio(out)]
///   dst: Streamp<T>,
///
///   other_parameter: u32,
/// }
/// impl<T: Copy> Block for MyBlock<T> {
///   fn work(…) … {
///   }
/// }
/// ```
///
/// Struct attributes:
/// * `new`: Generate `new()`, taking input streams and other args.
/// * `out`: Generate `out()`, returning all output streams.
/// * `crate`: Block is in the main Rustradio crate.
/// * `sync`: Block is "one in, one out" via `process_sync()` instead of
///   `work()`.
/// * `sync_tag`: Same as `sync`, but allow tag processing using
///   `process_sync_tags()`.
/// * `custom_name`: Call `custom_name()` instead of using the struct name, as
///   name.
/// * `noeof`: Don't generate `eof()` logic.
/// * `nevereof`: Generate `eof()` that always returns false.
///
/// Field attributes:
/// * `in`: Input stream.
/// * `out`: Output stream.
/// * `default`: Skip this field as arg for the `new()` function, and instead
///   default it.
///
#[proc_macro_derive(Block, attributes(rustradio))]
pub fn derive_block(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    // eprintln!("{:?}", input.generics.split_for_impl());
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let struct_name = input.ident;
    //eprintln!("struct name: {struct_name}");
    let name_str = struct_name.to_string();

    let data_struct = match input.data {
        Data::Struct(d) => d,
        _ => panic!("derive_block can only be used on structs"),
    };
    let fields_named = match data_struct.fields {
        Fields::Named(f) => f,
        // _ => return quote! { false }.into(),
        x => panic!("Fields is what? {x:?}"),
    };

    let path = match has_attr(&input.attrs, "crate", STRUCT_ATTRS) {
        true => quote! { crate },
        false => quote! { rustradio },
    };

    let mut fields_defaulted_ty = vec![];
    let fields_defaulted: std::collections::HashSet<String> = fields_named
        .named
        .iter()
        .filter(|field| has_attr(&field.attrs, "default", FIELD_ATTRS))
        .map(|field| {
            let field_name = field.ident.clone().unwrap();
            let ty = field.ty.clone();
            fields_defaulted_ty.push(quote! { #field_name: #ty::default() });
            field_name.to_string()
        })
        .collect();

    let mut set_eofs = vec![];
    let mut eof_checks = vec![];
    let mut extra = vec![];

    // TODO: surely there's a cleaner way.
    let mut ins = vec![]; // E.g. `src`.
    let mut insty = vec![]; // E.g. `src: Streamp<Complex>`
    let mut outs = vec![]; // E.g. `dst`.
    let mut outsty = vec![]; // E.g. Stream<Float>
    let mut in_types = vec![]; // E.g. Complex.
    let mut in_name_types = vec![]; // E.g. `a: Complex`.
    let mut out_types = vec![]; // E.g. Float.
    let mut other = vec![];
    let mut otherty = vec![];
    fields_named.named.iter().for_each(|field| {
        let field_name = field.ident.clone().unwrap();
        let found_in = has_attr(&field.attrs, "in", FIELD_ATTRS);
        let found_out = has_attr(&field.attrs, "out", FIELD_ATTRS);
        match (found_in, found_out) {
            (true, true) => panic!("Field {field_name} marked both as input and output stream."),
            (false, false) => {
                // panic!("Field {field:?} marked neither input nor output");
                if !fields_defaulted.contains(&field_name.to_string()) {
                    let ty = field.ty.clone();
                    other.push(field_name.clone());
                    otherty.push(quote! { #field_name: #ty});
                }
            }
            (false, true) => {
                set_eofs.push(quote! { self.#field_name.set_eof(); });
                outs.push(field_name.clone());
                let ty = field.ty.clone();
                outsty.push(quote! { #ty });
                let inner = inner_type(&ty);
                out_types.push(quote! { #inner });
            }
            (true, false) => {
                eof_checks.push(quote! { self.#field_name.eof() });
                ins.push(field_name.clone());
                let ty = field.ty.clone();
                insty.push(quote! { #field_name: #ty });
                let inner = inner_type(&ty);
                in_types.push(quote! { #inner });
                in_name_types.push(quote! { #field_name: #inner });
            }
        };
    });

    if has_attr(&input.attrs, "new", STRUCT_ATTRS) {
        extra.push(quote! {
            impl #impl_generics #struct_name #ty_generics #where_clause {
                pub fn new(#(#insty,)*#(#otherty),*) -> Self {
                    Self {
                    #(#ins,)*
                    #(#outs: #path::Stream::newp(),)*
                    #(#other,)*
                    #(#fields_defaulted_ty,)*
                    }
                }
            }
        });
    }
    if has_attr(&input.attrs, "out", STRUCT_ATTRS) {
        extra.push(quote! {
            impl #impl_generics #struct_name #ty_generics #where_clause {
                pub fn out(&self) -> (#(#outsty),*) {
                    (#(self.#outs.clone()),*)
                }
            }
        });
    }

    // Support sync blocks.
    // TODO: no way this works with anything more than two inputs, and one output.
    if has_attr(&input.attrs, "sync", STRUCT_ATTRS)
        || has_attr(&input.attrs, "sync_tag", STRUCT_ATTRS)
    {
        let first = ins[0].clone();
        let rest = &ins[1..];
        let it = if ins.len() == 1 {
            quote! { #first.iter().take(n) }
        } else {
            quote! { #first.iter().take(n)#(.zip(#rest.iter()))* }
        };
        if has_attr(&input.attrs, "sync", STRUCT_ATTRS) {
            extra.push(quote! {
                impl #impl_generics #struct_name #ty_generics #where_clause {
                    fn process_sync_tags(&mut self, #(#in_name_types,)* tags: &[#path::stream::Tag]) -> (#(#out_types,)* Vec<#path::stream::Tag>) {
                        (self.process_sync(#(#ins,)*), tags.to_vec())
                    }
                }
            });
        }
        extra.push(quote! {
            impl #impl_generics #path::block::Block for #struct_name #ty_generics #where_clause {
                fn work(&mut self) -> Result<#path::block::BlockRet, #path::Error> {
                    #(let #ins = self.#ins.clone();
                      let #ins = #ins.read_buf()?;)*
                    let mut tags = #first.1;
                    #(let #ins = #ins.0;)*

                    // Clamp n to be no more than the input available.
                    let n = [#(#ins.len()),*].iter().fold(usize::MAX, |min, &x|min.min(x));
                    if n ==  0 {
                        return Ok(#path::block::BlockRet::Noop);
                    }
                    #(let #outs = self.#outs.clone();
                      let mut #outs = #outs.write_buf()?;)*

                    // Clamp n to be no more than output space.
                    let n = [#(#outs.len()),*].iter().fold(n, |min, &x|min.min(x));
                    let mut otags = Vec::new();
                    let it = #it.enumerate().map(|(pos, (#(#ins),*))| {
                        // let (s, ts) = self.process_sync_tags(#(*#ins),*,&tags);
                        let (s, ts) = self.process_sync_tags(#(*#ins),*,&[]);
                        for tag in ts {
                            otags.push(#path::stream::Tag::new(pos, tag.key().into(), tag.val().clone()));
                        }
                        s
                    });
                    for (samp, w) in it.zip(#(#outs.slice().iter_mut())*) {
                        *w = samp;
                    }
                    #(#ins.consume(n);)*
                    #(#outs.produce(n, &otags);)*
                    Ok(#path::block::BlockRet::Ok)
                }
            }
        });
    }

    {
        let nameval = if has_attr(&input.attrs, "custom_name", STRUCT_ATTRS) {
            quote! { self.custom_name() }
        } else {
            quote! { #name_str }
        };
        extra.push(quote! {
            impl #impl_generics #path::block::BlockName for #struct_name #ty_generics #where_clause {
            fn block_name(&self) -> &str {
                #nameval
            }
        }
        });
    }

    extra.push(match (ins.is_empty(), has_attr(&input.attrs, "noeof", STRUCT_ATTRS), has_attr(&input.attrs, "nevereof", STRUCT_ATTRS)) {
        (true, _, _) => quote! {
            impl #impl_generics #path::block::BlockEOF for #struct_name #ty_generics #where_clause {}
        },
        (false, false, false) => quote! {
                 impl #impl_generics #path::block::BlockEOF for #struct_name #ty_generics #where_clause {
                    fn eof(&mut self) -> bool {
                        if true #(&&#eof_checks)* {
                            #(#set_eofs)*
                            true
                        } else {
                            false
                        }
                    }
                 }
            },
        (false, true, false) => quote! {},
        (false, false, true) => quote! {
            impl #impl_generics #path::block::BlockEOF for #struct_name #ty_generics #where_clause {}
        },
        (false, true, true) => panic!("Providing noeof and nevereof is not valid"),
    });

    TokenStream::from(quote! { #(#extra)* })
}
/* vim: textwidth=80
 */
