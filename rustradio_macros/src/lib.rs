//! Derive macros for rustradio.
//!
//! Most blocks should derive from this macro.
use proc_macro::TokenStream;

use quote::quote;
use syn::{Attribute, Data, DeriveInput, Fields, Meta, parse_macro_input};

use paste::paste;

// TODO: change this macro to take `n` instead of a list of temporary variable
// names.
macro_rules! unzip_n {
    ($iter:expr, $($name:ident),+) => {{
        $(let paste!(mut [<agg_ $name>]) = Vec::new();)*
        for tuple in $iter {
            let ($($name),+) = tuple;
            paste! {
                $([<agg_ $name>].push($name);)+
            }
        }
        ($(paste! { [<agg_ $name>] }),+)
    }};
}

static STRUCT_ATTRS: &[&str] = &["new", "crate", "sync", "sync_tag", "custom_name", "noeof"];
static FIELD_ATTRS: &[&str] = &["in", "out", "default", "into"];

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
/// E.g. given ReadStream<Float>, return Float.
fn inner_type(ty: &syn::Type) -> (&syn::PathSegment, &syn::Type) {
    if let syn::Type::Path(p) = &ty {
        let segment = p.path.segments.last().unwrap();
        //assert_eq!(segment.ident, "Streamp");
        if let syn::PathArguments::AngleBracketed(angle_bracketed_args) = &segment.arguments {
            for arg in &angle_bracketed_args.args {
                if let syn::GenericArgument::Type(ty) = arg {
                    return (segment, ty);
                }
            }
        }
    }
    panic!(
        "Tried to get the inner type of a non-generic, probably non-Stream: {}",
        quote! { #ty }
    )
}

/// Return the outer type of a generic type.
///
/// E.g. given Vec<Float>, return Vec.
///
/// Since a type can be a bit complicated, maybe it's fair to clarify that the
/// last part of the type path has its path arguments removed.
fn outer_type(ty: &syn::Type) -> syn::Type {
    //eprintln!("Finding outer type of {}", quote! { #ty }.to_string());
    //eprintln!("  {:?}", ty);
    let mut ty = ty.clone();
    if let syn::Type::Path(p) = &mut ty {
        let n = p.path.segments.len();
        let segment: &mut syn::PathSegment = &mut p.path.segments[n - 1];
        segment.arguments = syn::PathArguments::None;
    }
    ty
}

/// Block derive macro.
///
/// Most blocks should derive from `Block`. Example use:
///
/// ```
/// use rustradio::{Result, Error};
/// use rustradio::block::{Block, BlockRet};
/// use rustradio::stream::{ReadStream, WriteStream};
/// #[derive(rustradio_macros::Block)]
/// #[rustradio(new)]
/// pub struct MyBlock<T: Copy> {
///   #[rustradio(in)]
///   src: ReadStream<T>,
///   #[rustradio(out)]
///   dst: WriteStream<T>,
///
///   other_parameter: u32,
/// }
/// impl<T: Copy> Block for MyBlock<T> {
///   fn work(&mut self) -> Result<BlockRet> {
///     todo!()
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
///
/// Field attributes:
/// * `in`: Input stream.
/// * `out`: Output stream.
/// * `default`: Skip this field as arg for the `new()` function, and instead
///   default it.
/// * `into`: When the `new()` function is generated, let non-stream values
///   accept anything `.into()`-convertable into the given type, not just the
///   generated type directly.
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

    // fields_defaulted_ty: The initializer expression for the field.
    let fields_defaulted_ty: Vec<_> = fields_named
        .named
        .iter()
        .filter(|field| has_attr(&field.attrs, "default", FIELD_ATTRS))
        .map(|field| {
            let field_name = field.ident.clone().unwrap();
            let ty = outer_type(&field.ty);
            quote! { #field_name: #ty::default() }
        })
        .collect();

    // Create vec of useful input expressions.
    //
    // Elements are of the form:
    // * in_names:          src
    // * in_name_types:     src: ReadStream<Complex>
    // * inval_name_types:  src: Complex
    // * in_tag_names:      src_tag
    // * intag_name_types:  src_tag: &'a [Vec]   or   src_tag: &[Vec]
    //
    // Only the first tag has lifetime marking, if it's a `sync` block. If it's
    // a `sync_tag` block they're all lifetimed marked, though.
    let (in_names, in_name_types, inval_name_types, in_tag_names, intag_name_types) = unzip_n![
        fields_named
            .named
            .iter()
            .filter(|field| has_attr(&field.attrs, "in", FIELD_ATTRS))
            .enumerate()
            .map(|(i, field)| {
                let (_, inner) = inner_type(&field.ty);
                let ty = field.ty.clone();
                let field_name = field.ident.clone().unwrap();
                let tagname: syn::Ident = syn::parse_str(&format!("{field_name}_tag")).unwrap();
                (
                    field_name.clone(),
                    quote! { #field_name: #ty },
                    quote! { #field_name: #inner },
                    quote! { #tagname },
                    if i == 0 || !has_attr(&input.attrs, "sync", STRUCT_ATTRS) {
                        quote! { #tagname: &'a [#path::stream::Tag] }
                    } else {
                        quote! { #tagname: &[#path::stream::Tag] }
                    },
                )
            }),
        a,
        b,
        c,
        d,
        e
    ];

    // Create vec of useful out expressions.
    //
    // Elements are of the form:
    // * out_names:          dst
    // * out_names_samp:     dst_sample
    // * out_types:          WriteStream<Complex>
    // * outval_types:       Complex
    let (out_names, out_names_samp, _out_types, out_stream_type, out_factory, outval_types) = unzip_n![
        fields_named
            .named
            .iter()
            .filter(|field| has_attr(&field.attrs, "out", FIELD_ATTRS))
            .map(|field| {
                let (outer, inner) = inner_type(&field.ty);
                //eprintln!("{:?}", outer.ident);
                let ty = field.ty.clone();
                let field_name = field.ident.clone().unwrap();
                let samp_name: syn::Ident =
                    syn::parse_str(&format!("{field_name}_sample")).unwrap();
                (
                    field_name.clone(),
                    samp_name,
                    quote! { #ty },
                    if outer.ident == "NCWriteStream" {
                        quote! { #path::stream::NCReadStream<#inner> }
                    } else {
                        quote! { #path::stream::ReadStream<#inner> }
                    },
                    if outer.ident == "NCWriteStream" {
                        quote! { #path::stream::new_nocopy_stream() }
                    } else {
                        quote! { #path::stream::new_stream() }
                    },
                    quote! { #inner },
                )
            }),
        a,
        b,
        c,
        d,
        e,
        f
    ];

    // Ensure no field is marked both input and output.
    for field in &fields_named.named {
        assert!(
            !(has_attr(&field.attrs, "in", FIELD_ATTRS)
                && has_attr(&field.attrs, "out", FIELD_ATTRS)),
            "Field {} marked as both input and output stream",
            field.ident.clone().unwrap()
        );
    }

    // Create vec of fields that are not input, output, nor defaulted.

    let other_names_no_into: Vec<_> = fields_named
        .named
        .iter()
        .filter(|field| {
            !has_attr(&field.attrs, "in", FIELD_ATTRS)
                && !has_attr(&field.attrs, "out", FIELD_ATTRS)
                && !has_attr(&field.attrs, "into", FIELD_ATTRS)
                && !has_attr(&field.attrs, "default", FIELD_ATTRS)
        })
        .map(|field| {
            let field_name = field.ident.clone().unwrap();
            field_name.clone()
        })
        .collect();
    let other_name_types: Vec<_> = fields_named
        .named
        .iter()
        .filter(|field| {
            !has_attr(&field.attrs, "in", FIELD_ATTRS)
                && !has_attr(&field.attrs, "out", FIELD_ATTRS)
                && !has_attr(&field.attrs, "default", FIELD_ATTRS)
        })
        .map(|field| {
            let field_name = field.ident.clone().unwrap();
            let ty = field.ty.clone();
            if has_attr(&field.attrs, "into", FIELD_ATTRS) {
                let gen_name: syn::Type =
                    syn::parse_str(&format!("Into{field_name}")).expect("creating Into type");
                quote! { #field_name: #gen_name}
            } else {
                quote! { #field_name: #ty}
            }
        })
        .collect();
    let (other_into_names, other_into_types) = unzip_n![
        fields_named
            .named
            .iter()
            .filter(|field| has_attr(&field.attrs, "into", FIELD_ATTRS))
            .map(|field| {
                let ty = field.ty.clone();
                let field_name = field.ident.clone().unwrap();
                //let gen_name = prefixed_into_type(&ty);
                let gen_name: syn::Type = syn::parse_str(&format!("Into{field_name}")).unwrap();
                (quote! { #field_name }, quote! { #gen_name: Into<#ty> })
            }),
        a,
        b
    ];

    let mut extra = vec![]; // If requested, generate some extra code.

    // Create new(), if requested.
    if has_attr(&input.attrs, "new", STRUCT_ATTRS) {
        extra.push(quote! {
            impl #impl_generics #struct_name #ty_generics #where_clause {
                /// Create a new block.
                ///
                /// The arguments to this function are the mandatory input
                /// streams, and the mandatory parameters.
                ///
                /// The return values are the block itself, plus any mandatory
                /// output streams.
                ///
                /// This function is automatically generated by a macro.
                pub fn new #(<#other_into_types>),*(#(#in_name_types,)*#(#other_name_types),*) -> (Self #(,#out_stream_type)*) {
                    #(let #out_names = #out_factory;)*
                    (Self {
                    #(#in_names,)*
                    #(#out_names: #out_names.0,)*
                    #(#other_into_names: #other_into_names.into(),)*
                    #(#other_names_no_into,)*
                    #(#fields_defaulted_ty,)*
                    }#(,#out_names.1)*)
                }
            }
        });
    }

    // Support sync blocks.
    if has_attr(&input.attrs, "sync", STRUCT_ATTRS)
        || has_attr(&input.attrs, "sync_tag", STRUCT_ATTRS)
    {
        let first = in_names[0].clone();
        let rest = &in_names[1..];
        let it = if in_names.len() == 1 {
            quote! { #first.iter().take(n) }
        } else {
            quote! { #first.iter().take(n)#(.zip(#rest.iter()))* }
        };
        if has_attr(&input.attrs, "sync", STRUCT_ATTRS) {
            let first_tags = &in_tag_names[0];
            extra.push(quote! {
                impl #impl_generics #struct_name #ty_generics #where_clause {
                    fn process_sync_tags<'a>(&mut self, #(#inval_name_types, #intag_name_types,)*) -> (#(#outval_types,)* std::borrow::Cow<'a, [#path::stream::Tag]>) {
                        let (#(#out_names),*) = self.process_sync(#(#in_names,)*);
                        (#(#out_names,)*std::borrow::Cow::Borrowed(#first_tags))
                    }
                }
            });
        }
        extra.push(quote! {
            impl #impl_generics #path::block::Block for #struct_name #ty_generics #where_clause {
                fn work(&mut self) -> #path::Result<#path::block::BlockRet> {
                    #( let #in_names = self.#in_names.read_buf()?;)*
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

                    let mut otags = Vec::new();
                    let empty_tags = true #(&&#in_tag_names.is_empty())*;
                    let it = #it.enumerate().map(|(pos, (#(#in_names),*))| {
                        if empty_tags {
                            // Fast path for input without tags.
                            // There may be opportunity to deduplicate some of
                            // the next couple of lines with the !empty_tags
                            // case.
                            let (#(#out_names,)* ts) = self.process_sync_tags(#(*#in_names, &[]),*);
                            for tag in ts.iter() {
                                otags.push(#path::stream::Tag::new(pos, tag.key(), tag.val().clone()));
                            }
                            (#(#out_names),*)
                        } else {
                            // TODO: This tag filtering is quite expensive.
                            #(let #in_tag_names: Vec<_> = #in_tag_names.iter()
                              .filter(|t| t.pos() == pos)
                              .map(|t| #path::stream::Tag::new(0, t.key().to_string(), t.val().clone()))
                              .collect();)*
                            let (#(#out_names,)* ts) = self.process_sync_tags(#(*#in_names, &#in_tag_names),*);
                            for tag in ts.iter() {
                                otags.push(#path::stream::Tag::new(pos, tag.key(), tag.val().clone()));
                            }
                            (#(#out_names),*)
                        }
                    });
                    for ((#(#out_names_samp),*), #(#out_names,)*) in itertools::izip!(it, #(#out_names.slice().iter_mut()),*) {
                        (#(*#out_names),*) = (#(#out_names_samp),*);
                    }
                    #(#in_names.consume(n);)*
                    #(#out_names.produce(n, &otags);)*
                    Ok(#path::block::BlockRet::Again)
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

    extra.push(match (in_names.is_empty(), has_attr(&input.attrs, "noeof", STRUCT_ATTRS)) {
        // No inputs.
        (true, _) => quote! {
            impl #impl_generics #path::block::BlockEOF for #struct_name #ty_generics #where_clause {
                fn eof(&mut self) -> bool {
                    false
                }
            }
        },
        // Has inputs, eof generation (implicitly) requested.
        (false, false) => quote! {
                 impl #impl_generics #path::block::BlockEOF for #struct_name #ty_generics #where_clause {
                    fn eof(&mut self) -> bool {
                        if true #(&&self.#in_names.eof())* {
                            true
                        } else {
                            false
                        }
                    }
                 }
            },
        // Has inputs, noeof requested. The block will have to manually
        // implement eof.
        (false, true) => quote! {},
    });

    TokenStream::from(quote! { #(#extra)* })
}
/* vim: textwidth=80
 */
