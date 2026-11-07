mod route;
mod validated_schema;

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_attribute]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as route::Args);
    let fun = parse_macro_input!(item as syn::ItemFn);
    route::expand(&args, &fun)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn validated_schema(attr: TokenStream, item: TokenStream) -> TokenStream {
    let patch = match parse_macro_input!(attr as Option<syn::Ident>) {
        None => false,
        Some(flag) if flag == "patch" => true,
        Some(flag) => {
            return syn::Error::new_spanned(flag, "validated_schema accepts only patch")
                .into_compile_error()
                .into();
        }
    };
    let item = parse_macro_input!(item as syn::Item);
    validated_schema::expand(item, patch)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(Ctor, attributes(ctor))]
pub fn derive_ctor(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_ctor(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_ctor(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let syn::Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(input, "Ctor requires a struct"));
    };
    let syn::Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(input, "Ctor requires named fields"));
    };
    let mut fn_name = quote::format_ident!("new");
    for attr in &input.attrs {
        if attr.path().is_ident("ctor") {
            fn_name = attr.parse_args()?;
        }
    }
    let idents: Vec<_> = fields.named.iter().map(|f| &f.ident).collect();
    let types: Vec<_> = fields.named.iter().map(|f| &f.ty).collect();
    let arguments_expect =
        (idents.len() > 7).then(|| quote!(#[expect(clippy::too_many_arguments)]));
    let option_expect = types
        .iter()
        .any(|ty| option_inner(ty).is_some_and(|inner| option_inner(inner).is_some()))
        .then(|| quote!(#[expect(clippy::option_option)]));
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            #arguments_expect
            #option_expect
            pub(crate) fn #fn_name(#(#idents: #types),*) -> Self {
                Self { #(#idents),* }
            }
        }
    })
}

fn ungroup(ty: &syn::Type) -> &syn::Type {
    match ty {
        syn::Type::Group(group) => ungroup(&group.elem),
        _ => ty,
    }
}

fn option_inner(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(path) = ungroup(ty) else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(inner) = arguments.args.first()? else {
        return None;
    };
    Some(inner)
}
