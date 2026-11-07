use std::collections::BTreeSet;

use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, Expr, Field, Item, Meta, Token, Type};

pub(crate) fn expand(mut item: Item, patch: bool) -> syn::Result<TokenStream> {
    let into_params = derives(&item, "IntoParams");
    if derives(&item, "Deserialize") {
        deny_unknown_fields(&mut item)?;
    }
    if patch {
        patch_fields(&mut item)?;
    }
    let fields: Vec<&mut Field> = match &mut item {
        Item::Struct(item) => item.fields.iter_mut().collect(),
        Item::Enum(item) => item
            .variants
            .iter_mut()
            .flat_map(|variant| &mut variant.fields)
            .collect(),
        _ => {
            return Err(syn::Error::new_spanned(
                item,
                "validated_schema requires a struct or enum",
            ));
        }
    };
    let mut checks = Vec::new();
    for field in fields {
        checks.extend(annotate_field(field, into_params)?);
    }
    Ok(quote! {
        #item
        #(const _: fn() -> ::std::borrow::Cow<'static, str> = #checks::rule;)*
    })
}

fn derives(item: &Item, name: &str) -> bool {
    let attrs = match item {
        Item::Struct(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        _ => return false,
    };
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive"))
        .any(|attr| {
            attr.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated)
                .is_ok_and(|paths| {
                    paths.iter().any(|path| {
                        path.segments
                            .last()
                            .is_some_and(|segment| segment.ident == name)
                    })
                })
        })
}

fn deny_unknown_fields(item: &mut Item) -> syn::Result<()> {
    let attrs = match item {
        Item::Struct(item) => &mut item.attrs,
        Item::Enum(item) => &mut item.attrs,
        _ => return Ok(()),
    };
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
        let features = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        if features
            .iter()
            .any(|feature| feature.path().is_ident("deny_unknown_fields"))
        {
            return Ok(());
        }
    }
    attrs.push(syn::parse_quote!(#[serde(deny_unknown_fields)]));
    Ok(())
}

fn patch_fields(item: &mut Item) -> syn::Result<()> {
    let Item::Struct(target) = item else {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "patch requires a struct",
        ));
    };
    let mut three_state = false;
    for field in &mut target.fields {
        let Some(inner) = super::option_inner(&field.ty) else {
            continue;
        };
        if feature_names(&field.attrs, "serde")?.contains("with") {
            continue;
        }
        if super::option_inner(inner).is_some() {
            three_state = true;
            field.attrs.push(
                syn::parse_quote!(#[serde(default, with = "serde_with::rust::double_option")]),
            );
        } else {
            field.attrs.push(
                syn::parse_quote!(#[serde(default, with = "serde_with::rust::unwrap_or_skip")]),
            );
            append_features(
                &mut field.attrs,
                "schema",
                &[Feature::new("nullable", quote!(nullable = false))],
            )?;
        }
    }
    if three_state {
        target
            .attrs
            .push(syn::parse_quote!(#[expect(clippy::option_option)]));
    }
    Ok(())
}

fn annotate_field(field: &mut Field, into_params: bool) -> syn::Result<Vec<syn::Path>> {
    let mut features = Vec::new();
    let mut checks = Vec::new();
    for attr in field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("garde"))
    {
        let rules = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        collect_rules(&rules, &field.ty, &mut features, &mut checks)?;
    }
    if field.ident.is_none() {
        return Ok(checks);
    }
    if into_params {
        append_features(&mut field.attrs, "param", &features)?;
    }
    if !checks.is_empty() {
        let rule = quote!(json!(([#(#checks::rule()),*].join(". "))));
        features.push(Feature::new(
            "extensions",
            quote!(extensions(("x-rule" = #rule))),
        ));
    }
    append_features(&mut field.attrs, "schema", &features)?;
    Ok(checks)
}

fn collect_rules(
    rules: &Punctuated<Meta, Token![,]>,
    ty: &Type,
    features: &mut Vec<Feature>,
    checks: &mut Vec<syn::Path>,
) -> syn::Result<()> {
    for rule in rules {
        match rule {
            Meta::List(rule) if rule.path.is_ident("custom") => {
                checks.push(check_path(&syn::parse2(rule.tokens.clone())?)?);
            }
            Meta::List(rule) if rule.path.is_ident("length") => {
                let bounds =
                    rule.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
                let [min, max] = bounds_of(&bounds);
                match kind(ty) {
                    Kind::String => {
                        push_bound(features, "min_length", min);
                        push_bound(features, "max_length", max);
                    }
                    Kind::Array => {
                        push_bound(features, "min_items", min);
                        push_bound(features, "max_items", max);
                    }
                    Kind::Other => {}
                }
            }
            Meta::List(rule) if rule.path.is_ident("range") => {
                let bounds =
                    rule.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
                let [min, max] = bounds_of(&bounds);
                push_bound(features, "minimum", min);
                push_bound(features, "maximum", max);
            }
            Meta::List(rule) if rule.path.is_ident("pattern") && kind(ty) == Kind::String => {
                let expr: Expr = syn::parse2(rule.tokens.clone())?;
                if matches!(&expr, Expr::Lit(value) if matches!(value.lit, syn::Lit::Str(_))) {
                    features.push(Feature::new("pattern", quote!(pattern = #expr)));
                }
            }
            Meta::Path(rule) if rule.is_ident("email") && kind(ty) == Kind::String => {
                features.push(Feature::new("format", quote!(format = Email)));
            }
            Meta::Path(rule) if rule.is_ident("required") => {
                features.push(Feature::new("required", quote!(required = true)));
            }
            Meta::List(rule) if rule.path.is_ident("inner") => {
                let rules =
                    rule.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
                match super::option_inner(ty) {
                    Some(inner) => collect_rules(&rules, inner, features, checks)?,
                    None => collect_rules(&rules, ty, &mut Vec::new(), checks)?,
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn check_path(expr: &Expr) -> syn::Result<syn::Path> {
    let callee = match expr {
        Expr::Call(call) => &*call.func,
        expr => expr,
    };
    match callee {
        Expr::Path(callee) => Ok(callee.path.clone()),
        _ => Err(syn::Error::new_spanned(
            expr,
            "custom validator must be a path to a function with a rule module",
        )),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    String,
    Array,
    Other,
}

fn kind(ty: &Type) -> Kind {
    if let Some(inner) = super::option_inner(ty) {
        return kind(inner);
    }
    let Type::Path(ty) = super::ungroup(ty) else {
        return Kind::Other;
    };
    let Some(segment) = ty.path.segments.last() else {
        return Kind::Other;
    };
    if segment.ident == "String" || segment.ident == "str" {
        Kind::String
    } else if matches!(
        segment.ident.to_string().as_str(),
        "Vec" | "VecDeque" | "HashSet" | "BTreeSet" | "IndexSet"
    ) {
        Kind::Array
    } else {
        Kind::Other
    }
}

fn bounds_of(rules: &Punctuated<Meta, Token![,]>) -> [Option<syn::LitInt>; 2] {
    ["min", "max"].map(|name| {
        rules.iter().find_map(|rule| match rule {
            Meta::NameValue(rule) if rule.path.is_ident(name) => match &rule.value {
                Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(int),
                    ..
                }) => Some(syn::LitInt::new(int.base10_digits(), int.span())),
                _ => None,
            },
            _ => None,
        })
    })
}

fn push_bound(features: &mut Vec<Feature>, name: &'static str, value: Option<syn::LitInt>) {
    if let Some(value) = value {
        let ident = syn::Ident::new(name, proc_macro2::Span::call_site());
        features.push(Feature::new(name, quote!(#ident = #value)));
    }
}

struct Feature {
    name: &'static str,
    tokens: TokenStream,
}

impl Feature {
    fn new(name: &'static str, tokens: TokenStream) -> Self {
        Self { name, tokens }
    }
}

fn append_features(
    attrs: &mut Vec<Attribute>,
    attribute: &str,
    features: &[Feature],
) -> syn::Result<()> {
    let existing = feature_names(attrs, attribute)?;
    let features: Vec<_> = features
        .iter()
        .filter(|feature| !existing.contains(feature.name))
        .map(|feature| &feature.tokens)
        .collect();
    if features.is_empty() {
        return Ok(());
    }
    let attribute = syn::Ident::new(attribute, proc_macro2::Span::call_site());
    attrs.push(syn::parse_quote!(#[#attribute(#(#features),*)]));
    Ok(())
}

fn feature_names(attrs: &[Attribute], attribute: &str) -> syn::Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident(attribute)) {
        let features = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for feature in features {
            if let Some(ident) = feature.path().get_ident() {
                names.insert(ident.to_string());
            }
        }
    }
    Ok(names)
}
