use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use syn::spanned::Spanned;

use crate::files;

#[derive(Default, Clone, Copy)]
pub(crate) struct Count {
    pub(crate) code: usize,
    pub(crate) tests: usize,
}

impl Count {
    fn add(&mut self, other: Count) {
        self.code += other.code;
        self.tests += other.tests;
    }
}

pub(crate) struct Report {
    pub(crate) crates: Vec<(String, Count)>,
    pub(crate) total: Count,
    pub(crate) parse_failures: usize,
}

pub(crate) fn scan(root: &Path) -> files::Result<Report> {
    let paths = files::rust(root, &["xtask"])?;

    let mut per_crate: BTreeMap<String, Count> = BTreeMap::new();
    let mut total = Count::default();
    let mut parse_failures = 0;

    for path in &paths {
        let (count, failed) = count_file(path)?;
        if failed {
            parse_failures += 1;
        }
        total.add(count);
        per_crate
            .entry(crate_name(root, path))
            .or_default()
            .add(count);
    }

    Ok(Report {
        crates: per_crate.into_iter().collect(),
        total,
        parse_failures,
    })
}

fn count_file(path: &Path) -> files::Result<(Count, bool)> {
    let source = files::read(path)?;
    let in_tests = path
        .components()
        .any(|c| c.as_os_str() == "tests" || c.as_os_str() == "benches");

    let Some(code_lines) = crate::comments::code_lines(&source) else {
        let nonblank = source.lines().filter(|l| !l.trim().is_empty()).count();
        let count = if in_tests {
            Count {
                code: 0,
                tests: nonblank,
            }
        } else {
            Count {
                code: nonblank,
                tests: 0,
            }
        };
        return Ok((count, true));
    };

    if in_tests {
        return Ok((
            Count {
                code: 0,
                tests: code_lines.len(),
            },
            false,
        ));
    }

    let Ok(file) = syn::parse_file(&source) else {
        return Ok((
            Count {
                code: code_lines.len(),
                tests: 0,
            },
            true,
        ));
    };

    let mut test_lines = BTreeSet::new();
    collect_test_lines(&file.items, &mut test_lines);

    let mut count = Count::default();
    for line in code_lines {
        if test_lines.contains(&line) {
            count.tests += 1;
        } else {
            count.code += 1;
        }
    }
    Ok((count, false))
}

fn collect_test_lines(items: &[syn::Item], out: &mut BTreeSet<usize>) {
    for item in items {
        if is_test_item(item) {
            let span = item.span();
            out.extend(span.start().line..=span.end().line);
        } else if let syn::Item::Mod(m) = item
            && let Some((_, inner)) = &m.content
        {
            collect_test_lines(inner, out);
        }
    }
}

pub(crate) fn is_test_item(item: &syn::Item) -> bool {
    item_attrs(item).is_some_and(has_test_attr)
}

fn has_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        if path.segments.last().is_some_and(|s| s.ident == "test") {
            return true;
        }
        if path.is_ident("cfg")
            && let syn::Meta::List(list) = &attr.meta
        {
            return list.tokens.to_string().contains("test");
        }
        false
    })
}

fn item_attrs(item: &syn::Item) -> Option<&[syn::Attribute]> {
    use syn::Item;
    let attrs = match item {
        Item::Const(x) => &x.attrs,
        Item::Enum(x) => &x.attrs,
        Item::ExternCrate(x) => &x.attrs,
        Item::Fn(x) => &x.attrs,
        Item::ForeignMod(x) => &x.attrs,
        Item::Impl(x) => &x.attrs,
        Item::Macro(x) => &x.attrs,
        Item::Mod(x) => &x.attrs,
        Item::Static(x) => &x.attrs,
        Item::Struct(x) => &x.attrs,
        Item::Trait(x) => &x.attrs,
        Item::TraitAlias(x) => &x.attrs,
        Item::Type(x) => &x.attrs,
        Item::Union(x) => &x.attrs,
        Item::Use(x) => &x.attrs,
        _ => return None,
    };
    Some(attrs)
}

fn crate_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .and_then(|rel| rel.components().next())
        .map_or_else(
            || "(root)".to_owned(),
            |c| c.as_os_str().to_string_lossy().into_owned(),
        )
}
