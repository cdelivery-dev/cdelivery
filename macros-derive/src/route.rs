use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};

pub(crate) struct Args {
    method: syn::Ident,
    path: syn::LitStr,
    output: Option<syn::Type>,
    responses: Vec<Response>,
}

struct Response {
    status: syn::Ident,
    body: Option<syn::Type>,
    content_type: Option<syn::LitStr>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        let path = input.parse()?;
        let mut output = None;
        let mut responses = Vec::new();
        while input.parse::<Option<syn::Token![,]>>()?.is_some() && !input.is_empty() {
            let fork = input.fork();
            let key = fork.parse::<syn::Ident>()?;
            if key == "output" {
                let key = input.parse::<syn::Ident>()?;
                input.parse::<syn::Token![=]>()?;
                if output.is_some() {
                    return Err(syn::Error::new_spanned(key, "duplicate output"));
                }
                output = Some(input.parse()?);
                continue;
            }
            responses.push(input.parse()?);
        }
        Ok(Self {
            method,
            path,
            output,
            responses,
        })
    }
}

impl Parse for Response {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let status = input.parse()?;
        let mut body = None;
        let mut content_type = None;
        if input.peek(syn::token::Paren) {
            let inner;
            syn::parenthesized!(inner in input);
            body = Some(inner.parse()?);
            if inner.parse::<Option<syn::Token![,]>>()?.is_some() {
                content_type = Some(inner.parse()?);
            }
        }
        Ok(Self {
            status,
            body,
            content_type,
        })
    }
}

fn split_path(path: &syn::LitStr) -> (syn::LitStr, Vec<(String, Option<String>)>) {
    let mut params = Vec::new();
    let cleaned = path
        .value()
        .split('/')
        .map(|segment| {
            let Some(inner) = segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            else {
                return segment.to_owned();
            };
            let (name, description) = match inner.split_once(':') {
                Some((name, description)) => (name, Some(description.to_owned())),
                None => (inner, None),
            };
            params.push((name.to_owned(), description));
            format!("{{{name}}}")
        })
        .collect::<Vec<_>>()
        .join("/");
    (syn::LitStr::new(&cleaned, path.span()), params)
}

fn extractor(ty: &syn::Type) -> Option<(&syn::Ident, &syn::Type)> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(inner) = args.args.first()? else {
        return None;
    };
    Some((&segment.ident, inner))
}

struct Extractors<'a> {
    path: Vec<&'a syn::Type>,
    query: Option<&'a syn::Type>,
    request_body: Option<TokenStream>,
    authorized: Option<&'a syn::Type>,
    state: Option<TokenStream>,
    authorized_binding: Option<TokenStream>,
    path_bindings: Vec<TokenStream>,
    query_binding: Option<TokenStream>,
    body_binding: Option<TokenStream>,
}

fn bindings(pat: &syn::Pat) -> syn::Result<Vec<TokenStream>> {
    let syn::Pat::TupleStruct(wrapper) = pat else {
        return Err(syn::Error::new_spanned(
            pat,
            "extractor pattern must be Wrapper(binding)",
        ));
    };
    let mut elems = wrapper.elems.iter();
    let (Some(inner), None) = (elems.next(), elems.next()) else {
        return Err(syn::Error::new_spanned(
            pat,
            "extractor pattern must bind exactly one value",
        ));
    };
    Ok(match inner {
        syn::Pat::Tuple(tuple) => tuple.elems.iter().map(|elem| quote!(#elem)).collect(),
        other => vec![quote!(#other)],
    })
}

fn extractors(fun: &syn::ItemFn) -> syn::Result<Extractors<'_>> {
    let mut extractors = Extractors {
        path: Vec::new(),
        query: None,
        request_body: None,
        authorized: None,
        state: None,
        authorized_binding: None,
        path_bindings: Vec::new(),
        query_binding: None,
        body_binding: None,
    };
    let mut user_input_seen = false;
    for input in &fun.sig.inputs {
        let syn::FnArg::Typed(arg) = input else {
            continue;
        };
        let Some((wrapper, inner)) = extractor(&arg.ty) else {
            continue;
        };
        match wrapper.to_string().as_str() {
            "State" => extractors.state = bindings(&arg.pat)?.pop(),
            "Path" => {
                user_input_seen = true;
                extractors.path = match inner {
                    syn::Type::Tuple(tuple) => tuple.elems.iter().collect(),
                    other => vec![other],
                };
                extractors.path_bindings = bindings(&arg.pat)?;
            }
            "Query" => {
                user_input_seen = true;
                extractors.query = Some(inner);
                extractors.query_binding = bindings(&arg.pat)?.pop();
            }
            "Body" => {
                user_input_seen = true;
                extractors.request_body = Some(quote!(request_body = #inner,));
                extractors.body_binding = bindings(&arg.pat)?.pop();
            }
            "Authorized" => {
                if user_input_seen {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "Authorized must precede user input extractors",
                    ));
                }
                extractors.authorized = Some(inner);
                let pat = &arg.pat;
                extractors.authorized_binding = Some(quote!(#pat));
            }
            _ => {}
        }
    }
    Ok(extractors)
}

fn synthesize(
    args: &Args,
    fun: &syn::ItemFn,
    extractors: &Extractors<'_>,
    operation: &syn::Type,
) -> syn::Result<syn::ItemFn> {
    let [response] = args.responses.as_slice() else {
        return Err(syn::Error::new_spanned(
            &fun.sig.ident,
            "a route with several success responses needs a handwritten body",
        ));
    };
    let state = extractors
        .state
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(&fun.sig, "route synthesis requires State"))?;
    if extractors.path_bindings.len() != extractors.path.len() {
        return Err(syn::Error::new_spanned(
            &fun.sig,
            "Path tuple must be destructured into one binding per parameter",
        ));
    }
    let authorized = &extractors.authorized_binding;
    let parts = extractors
        .path_bindings
        .iter()
        .chain(&extractors.query_binding)
        .chain(&extractors.body_binding);
    let call = quote!(#authorized.compose(&#state, (#(#parts,)*)).await?);
    let status = syn::Ident::new(
        &response.status.to_string().to_uppercase(),
        response.status.span(),
    );
    let body = match (&response.body, &response.content_type) {
        (Some(_), Some(_)) => quote!(crate::rest::response::attachment::<#operation>(#call)),
        (Some(_), None) if response.status == "ok" => quote!(Ok(axum::Json(#call))),
        (Some(_), None) => quote!(Ok((axum::http::StatusCode::#status, axum::Json(#call)))),
        (None, _) => quote!(#call; Ok(axum::http::StatusCode::#status)),
    };
    let mut fun = fun.clone();
    fun.block = Box::new(syn::parse_quote!({ #body }));
    Ok(fun)
}

fn output_check(args: &Args, operation: &syn::Type) -> syn::Result<TokenStream> {
    let artifact = artifact_responses(args)?;
    let output = if let Some(output) = &args.output {
        output_assertion(operation, output)
    } else {
        let [response] = args.responses.as_slice() else {
            return Err(syn::Error::new_spanned(
                operation,
                "operation route with multiple responses requires output = Type",
            ));
        };
        match (&response.body, &response.content_type) {
            (Some(body), None) => output_assertion(operation, body),
            (None, _) => output_assertion(operation, &syn::parse_quote!(crate::operation::Done)),
            (Some(_), Some(_)) => {
                output_assertion(operation, &syn::parse_quote!(crate::operation::Artifact))
            }
        }
    };
    let media = media_assertion(operation, artifact);
    Ok(quote! {
        #output
        #media
    })
}

fn artifact_responses(args: &Args) -> syn::Result<Option<&syn::LitStr>> {
    let Some(first) = args.responses.first() else {
        return Ok(None);
    };
    let artifact = first.content_type.as_ref();
    if let Some(response) = args.responses.iter().find(|response| {
        response.content_type.as_ref().map(syn::LitStr::value) != artifact.map(syn::LitStr::value)
    }) {
        return Err(syn::Error::new_spanned(
            &response.status,
            "operation success responses must use one media type",
        ));
    }
    Ok(artifact)
}

fn media_assertion(operation: &syn::Type, artifact: Option<&syn::LitStr>) -> TokenStream {
    let expected = if let Some(mime) = artifact {
        quote!(Some(#mime))
    } else {
        quote!(None)
    };
    quote! {
        const _: [(); 1] = [(); <#operation as crate::operation::Operation>::MEDIA
            .matches(#expected) as usize];
    }
}

fn output_assertion(operation: &syn::Type, output: &syn::Type) -> TokenStream {
    quote! {
        const _: fn() = || {
            fn output_matches<
                O: crate::operation::Operation<Output = #output>,
            >() {}
            output_matches::<#operation>();
        };
    }
}

pub(crate) fn expand(args: &Args, fun: &syn::ItemFn) -> syn::Result<TokenStream> {
    let (path, params) = split_path(&args.path);
    let extractors = extractors(fun)?;
    let operation = extractors
        .authorized
        .ok_or_else(|| syn::Error::new_spanned(&fun.sig, "route requires Authorized<O>"))?;
    if extractors.path.len() != params.len() {
        return Err(syn::Error::new_spanned(
            &args.path,
            "path params and Path extractor types mismatch",
        ));
    }
    let mut param_entries: Vec<TokenStream> = params
        .iter()
        .zip(&extractors.path)
        .map(|((name, description), ty)| {
            let description = description
                .as_ref()
                .map(|value| quote!(, description = #value));
            quote!((#name = #ty, Path #description))
        })
        .collect();
    if let Some(query) = extractors.query {
        param_entries.push(quote!(#query));
    }
    let params_clause = (!param_entries.is_empty()).then(|| quote!(params(#(#param_entries),*),));
    let request_body = &extractors.request_body;
    let responses = args.responses.iter().map(|response| {
        let status = syn::Ident::new(
            &response.status.to_string().to_uppercase(),
            response.status.span(),
        );
        let content_type = response
            .content_type
            .as_ref()
            .map(|value| quote!(, content_type = #value));
        let body = response.body.as_ref().map(|ty| quote!(, body = #ty));
        quote!((status = #status #content_type #body))
    });
    let output_check = output_check(args, operation)?;
    let fun = if fun.block.stmts.is_empty() {
        synthesize(args, fun, &extractors, operation)?
    } else {
        fun.clone()
    };
    let operation_id = quote!(
        operation_id = <#operation as crate::operation::Operation>::ID,
        description = <#operation as crate::operation::Operation>::DESCRIPTION,
    );
    let method = &args.method;
    let route = quote! {
        impl crate::rest::Route for #operation {
            const METHOD: &'static str = stringify!(#method);
            const PATH: &'static str = #path;
        }
    };
    Ok(quote! {
        #[utoipa::path(
            #method,
            path = #path,
            #operation_id
            #params_clause
            #request_body
            responses(
                #(#responses),*,
                (status = BAD_REQUEST, content_type = "application/problem+json", body = crate::error::Problem),
                (status = UNAUTHORIZED, content_type = "application/problem+json", body = crate::error::Problem),
                (status = FORBIDDEN, content_type = "application/problem+json", body = crate::error::Problem),
                (status = NOT_FOUND, content_type = "application/problem+json", body = crate::error::Problem),
                (status = METHOD_NOT_ALLOWED, content_type = "application/problem+json", body = crate::error::Problem),
                (status = CONFLICT, content_type = "application/problem+json", body = crate::error::Problem),
                (status = PAYLOAD_TOO_LARGE, content_type = "application/problem+json", body = crate::error::Problem),
                (status = UNSUPPORTED_MEDIA_TYPE, content_type = "application/problem+json", body = crate::error::Problem),
                (status = UNPROCESSABLE_ENTITY, content_type = "application/problem+json", body = crate::error::Problem),
                (status = INTERNAL_SERVER_ERROR, content_type = "application/problem+json", body = crate::error::Problem),
                (status = BAD_GATEWAY, content_type = "application/problem+json", body = crate::error::Problem),
                (status = SERVICE_UNAVAILABLE, content_type = "application/problem+json", body = crate::error::Problem),
                (status = GATEWAY_TIMEOUT, content_type = "application/problem+json", body = crate::error::Problem)
            )
        )]
        #fun
        #output_check
        #route
    })
}
