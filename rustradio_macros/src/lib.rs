//! Derive macros for rustradio.
//!
//! Most blocks should derive from this macro.
use proc_macro::TokenStream;

use quote::quote;
use syn::{parse_macro_input, Attribute, Data, DeriveInput, Fields, Meta};

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

static STRUCT_ATTRS: &[&str] = &[
    "new",
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
/// E.g. given ReadStream<Float>, return Float.
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
        "Tried to get the inner type of a non-generic, probably non-Stream: {}",
        quote! { #ty }.to_string()
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
    return ty;
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
///   src: ReadStream<T>,
///   #[rustradio(out)]
///   dst: WriteStream<T>,
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
    // Only the first tag has lifetime marking, if it's a sync block.
    let (in_names, in_name_types, inval_name_types, in_tag_names, intag_name_types) = unzip_n![
        fields_named
            .named
            .iter()
            .filter(|field| has_attr(&field.attrs, "in", FIELD_ATTRS))
            .enumerate()
            .map(|(i, field)| {
                let inner = inner_type(&field.ty);
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
    // * out_types_types:    WriteStream<Complex>
    // * outval_types:       Complex
    let (out_names, _out_types, outval_types) = unzip_n![
        fields_named
            .named
            .iter()
            .filter(|field| has_attr(&field.attrs, "out", FIELD_ATTRS))
            .map(|field| {
                let inner = inner_type(&field.ty);
                let ty = field.ty.clone();
                let field_name = field.ident.clone().unwrap();
                (field_name.clone(), quote! { #ty }, quote! { #inner })
            }),
        a,
        b,
        c
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
    let (other_names, other_name_types) = unzip_n![
        fields_named
            .named
            .iter()
            .filter(|field| !has_attr(&field.attrs, "in", FIELD_ATTRS)
                && !has_attr(&field.attrs, "out", FIELD_ATTRS)
                && !has_attr(&field.attrs, "default", FIELD_ATTRS))
            .map(|field| {
                let field_name = field.ident.clone().unwrap();
                let ty = field.ty.clone();
                (field_name.clone(), quote! { #field_name: #ty})
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
                pub fn new(#(#in_name_types,)*#(#other_name_types),*) -> (Self #(,#path::stream::ReadStream<#outval_types>)*) {
                    #(let #out_names = #path::stream::new_stream();)*
                    (Self {
                    #(#in_names,)*
                    #(#out_names: #out_names.0,)*
                    #(#other_names,)*
                    #(#fields_defaulted_ty,)*
                    }#(,#out_names.1)*)
                }
            }
        });
    }

    // Support sync blocks.
    // TODO: no way this works with anything more than two inputs, and one output.
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
                        (self.process_sync(#(#in_names,)*), std::borrow::Cow::Borrowed(#first_tags))
                    }
                }
            });
        }
        extra.push(quote! {
            impl #impl_generics #path::block::Block for #struct_name #ty_generics #where_clause {
                fn work(&mut self) -> Result<#path::block::BlockRet, #path::Error> {
                    #( let #in_names = self.#in_names.read_buf()?;)*
                    let mut tags = #first.1;
                    #(let #in_names = #in_names.0;)*

                    // Clamp n to be no more than the input available.
                    let n = [#(#in_names.len()),*].iter().fold(usize::MAX, |min, &x|min.min(x));
                    if n ==  0 {
                        return Ok(#path::block::BlockRet::Noop);
                    }
                    #( let mut #out_names = self.#out_names.write_buf()?;)*

                    // Clamp n to be no more than output space.
                    let n = [#(#out_names.len()),*].iter().fold(n, |min, &x|min.min(x));
                    let mut otags = Vec::new();
                    let it = #it.enumerate().map(|(pos, (#(#in_names),*))| {
                        // let (s, ts) = self.process_sync_tags(#(*#in_names),*,&tags);
                        //
                        // TODO: actually pass the tags.
                        let (s, ts) = self.process_sync_tags(#(*#in_names, &[]),*);
                        for tag in ts.iter() {
                            otags.push(#path::stream::Tag::new(pos, tag.key().into(), tag.val().clone()));
                        }
                        s
                    });
                    for (samp, w) in it.zip(#(#out_names.slice().iter_mut())*) {
                        *w = samp;
                    }
                    #(#in_names.consume(n);)*
                    #(#out_names.produce(n, &otags);)*
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

    extra.push(match (in_names.is_empty(), has_attr(&input.attrs, "noeof", STRUCT_ATTRS), has_attr(&input.attrs, "nevereof", STRUCT_ATTRS)) {
        // No inputs.
        (true, _, _) => quote! {
            impl #impl_generics #path::block::BlockEOF for #struct_name #ty_generics #where_clause {}
        },
        // Has inputs, eof generation (implicitly) requested.
        (false, false, false) => quote! {
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
        // Has inputs, no eof requested.
        (false, true, false) => quote! {},
        // Has inputs, "nevereof" requested.
        (false, false, true) => quote! {
            impl #impl_generics #path::block::BlockEOF for #struct_name #ty_generics #where_clause {}
        },
        // Invalid combination.
        (false, true, true) => panic!("Providing noeof and nevereof is not valid"),
    });

    TokenStream::from(quote! { #(#extra)* })
}
/* vim: textwidth=80
 */
