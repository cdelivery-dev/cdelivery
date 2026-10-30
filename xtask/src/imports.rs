use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;

use crate::{diagnostic, files, loc};

const DEPENDENCIES: [(&str, &[&str]); 8] = [
    ("agent", &["concept", "infra", "operation", "usecase"]),
    ("concept", &[]),
    ("infra", &["concept"]),
    ("mcp", &["concept", "operation"]),
    ("operation", &["concept", "usecase"]),
    ("rest", &["concept", "operation"]),
    ("ui", &["concept", "operation"]),
    ("usecase", &["concept", "infra"]),
];

const ALLOWANCES: [(&str, &str); 4] = [
    ("operation/mod.rs", "infra"),
    ("operation/typ/log.rs", "infra"),
    ("ui/auth.rs", "usecase"),
    ("ui/setup.rs", "usecase"),
];

pub(crate) struct Report {
    pub(crate) violations: Vec<diagnostic::Violation>,
    pub(crate) parse_failures: Vec<PathBuf>,
}

pub(crate) fn scan(root: &Path) -> files::Result<Report> {
    let src = root.join("cdelivery").join("src");
    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut parsed = Vec::new();

    for path in files::rust(&src, &[])? {
        let source = files::read(&path)?;
        let Ok(file) = syn::parse_file(&source) else {
            parse_failures.push(path);
            continue;
        };
        parsed.push((path, file));
    }
    let docker_methods: BTreeSet<_> = parsed
        .iter()
        .filter(|(path, _)| file_layer(&src, path) == Some("infra"))
        .flat_map(|(_, file)| docker_methods(file))
        .collect();
    let mut used = BTreeSet::new();
    for (path, file) in &parsed {
        let mut reach = Reach {
            layer: file_layer(&src, path),
            path,
            docker_methods: &docker_methods,
            out: &mut violations,
            references: BTreeMap::new(),
        };
        reach.items(&file.items);
        syn::visit::visit_file(&mut reach, file);
        reach.dependencies(&relative(&src, path), &mut used);
    }
    for (index, (file, target)) in ALLOWANCES.iter().enumerate() {
        if !used.contains(&index) {
            violations.push(diagnostic::Violation {
                path: src.join(file),
                line: 0,
                text: format!("stale allowance: {target} is no longer referenced"),
            });
        }
    }

    violations.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
    parse_failures.sort();
    Ok(Report {
        violations,
        parse_failures,
    })
}

fn docker_methods(file: &syn::File) -> BTreeSet<String> {
    let mut methods = BTreeSet::new();
    for item in &file.items {
        let syn::Item::Impl(item) = item else {
            continue;
        };
        let syn::Type::Path(typ) = item.self_ty.as_ref() else {
            continue;
        };
        if !typ.path.is_ident("Docker") {
            continue;
        }
        for item in &item.items {
            if let syn::ImplItem::Fn(method) = item
                && !matches!(method.vis, syn::Visibility::Inherited)
                && method.sig.receiver().is_some()
            {
                methods.insert(method.sig.ident.to_string());
            }
        }
    }
    methods
}

fn file_layer(src: &Path, path: &Path) -> Option<&'static str> {
    let rel = path.strip_prefix(src).ok()?;
    let mut components = rel.components();
    let first = components.next()?.as_os_str().to_str()?;
    components.next()?;
    layer(first)
}

fn layer(name: &str) -> Option<&'static str> {
    DEPENDENCIES
        .iter()
        .find_map(|&(layer, _)| (layer == name).then_some(layer))
}

fn relative(src: &Path, path: &Path) -> String {
    path.strip_prefix(src)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

struct Reach<'a> {
    layer: Option<&'static str>,
    path: &'a Path,
    docker_methods: &'a BTreeSet<String>,
    out: &'a mut Vec<diagnostic::Violation>,
    references: BTreeMap<&'static str, usize>,
}

impl Reach<'_> {
    fn flag(&mut self, line: usize, text: String) {
        self.out.push(diagnostic::Violation {
            path: self.path.to_path_buf(),
            line,
            text,
        });
    }

    fn reference(&mut self, target: &str, line: usize) {
        if let Some(layer) = layer(target) {
            self.references.entry(layer).or_insert(line);
        }
    }

    fn items(&mut self, items: &[syn::Item]) {
        for item in items {
            if loc::is_test_item(item) {
                continue;
            }
            match item {
                syn::Item::Use(item_use) => {
                    if item_use.leading_colon.is_none() {
                        let line = item_use.span().start().line;
                        self.tree(&item_use.tree, &mut Vec::new(), line);
                    }
                }
                syn::Item::Mod(module) => {
                    if let Some((_, inner)) = &module.content {
                        self.items(inner);
                    }
                }
                _ => {}
            }
        }
    }

    fn tree(&mut self, tree: &syn::UseTree, prefix: &mut Vec<String>, line: usize) {
        match tree {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                self.tree(&p.tree, prefix, line);
                prefix.pop();
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.tree(item, prefix, line);
                }
            }
            syn::UseTree::Name(name) => {
                self.leaf(prefix, &name.ident.to_string(), false, false, line);
            }
            syn::UseTree::Rename(rename) => {
                self.leaf(prefix, &rename.ident.to_string(), true, false, line);
            }
            syn::UseTree::Glob(_) => self.leaf(prefix, "self", false, true, line),
        }
    }

    fn leaf(&mut self, prefix: &[String], ident: &str, renamed: bool, glob: bool, line: usize) {
        let mut segments: Vec<&str> = prefix.iter().map(String::as_str).collect();
        if ident != "self" {
            segments.push(ident);
        }
        if let ["crate", target, ..] = segments.as_slice() {
            self.reference(target, line);
        }
        let Some(reason) = judge(&segments, renamed, glob, self.layer) else {
            return;
        };
        let mut rendered = segments.join("::");
        if glob {
            rendered.push_str("::*");
        }
        self.flag(line, format!("use {rendered} ({reason})"));
    }

    fn tokens(&mut self, stream: TokenStream) {
        let mut state = 0u8;
        for token in stream {
            state = match (&token, state) {
                (TokenTree::Ident(ident), _) if ident == "crate" => 1,
                (TokenTree::Punct(punct), 1 | 2) if punct.as_char() == ':' => state + 1,
                (TokenTree::Ident(ident), 3) => {
                    self.reference(&ident.to_string(), ident.span().start().line);
                    0
                }
                (TokenTree::Group(group), 3) => {
                    for inner in group.stream() {
                        if let TokenTree::Ident(ident) = inner {
                            self.reference(&ident.to_string(), ident.span().start().line);
                        }
                    }
                    self.tokens(group.stream());
                    0
                }
                (TokenTree::Group(group), _) => {
                    self.tokens(group.stream());
                    0
                }
                _ => 0,
            };
        }
    }

    fn dependencies(&mut self, file: &str, used: &mut BTreeSet<usize>) {
        let Some(layer) = self.layer else {
            return;
        };
        let allowed: &[&str] = DEPENDENCIES
            .iter()
            .find(|(name, _)| *name == layer)
            .map_or(&[], |&(_, deps)| deps);
        for (target, line) in std::mem::take(&mut self.references) {
            if target == layer || allowed.contains(&target) {
                continue;
            }
            if let Some(index) = ALLOWANCES
                .iter()
                .position(|(path, dep)| *path == file && *dep == target)
            {
                used.insert(index);
            } else {
                self.flag(
                    line,
                    format!("crate::{target} ({layer} must not depend on {target})"),
                );
            }
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for Reach<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        if !loc::is_test_item(item) {
            syn::visit::visit_item(self, item);
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if self.layer == Some("usecase") {
            let method = call.method.to_string();
            if self.docker_methods.contains(&method) {
                self.flag(
                    call.method.span().start().line,
                    format!(
                        ".{method}() (usecase must not drive the docker client; go through infra)"
                    ),
                );
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path.leading_colon.is_none() {
            let mut segments = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string());
            match (segments.next().as_deref(), segments.next()) {
                (Some("crate"), Some(target)) => {
                    self.reference(&target, path.span().start().line);
                }
                (Some("docker"), Some(_)) if self.layer == Some("usecase") => self.flag(
                    path.span().start().line,
                    "docker:: path (usecase must not name docker types; go through infra)"
                        .to_owned(),
                ),
                _ => {}
            }
        }
        syn::visit::visit_path(self, path);
    }
}

fn judge(
    segments: &[&str],
    renamed: bool,
    glob: bool,
    current: Option<&str>,
) -> Option<&'static str> {
    let current = current?;
    let first = segments.first()?;
    if *first == "super" || *first == "self" {
        return Some("sibling: drop the use, reference inline via super::");
    }
    if layer(first).is_some() {
        return Some("layer path must start with crate::");
    }
    if current == "usecase" && *first == "docker" {
        return Some("usecase must not import the docker crate; go through infra");
    }
    if *first != "crate" {
        return None;
    }
    let second = segments.get(1)?;
    layer(second)?;
    if glob {
        return Some("no glob imports of a layer");
    }
    if renamed {
        return Some("no renaming a layer");
    }
    if segments.len() > 2 {
        return Some("import the layer only, qualify at the call site");
    }
    if current == *second {
        return Some("own layer: drop the use, reference inline via super::");
    }
    None
}
