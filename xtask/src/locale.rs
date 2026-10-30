use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::{diagnostic, files};

const ENUM_LOCALES: [(&str, &str, &str); 6] = [
    ("cdelivery/src/error.rs", "ErrorKind", "error"),
    ("cdelivery/src/error.rs", "SpecProblem", "spec"),
    ("cdelivery/src/error.rs", "CertProblem", "cert"),
    ("cdelivery/src/error.rs", "AcmeProblem", "acme"),
    ("cdelivery/src/error.rs", "InstanceProblem", "instance"),
    ("insight-wire/src/event.rs", "Kind", "event"),
];

const LANGUAGES: [&str; 8] = ["en", "zh-CN", "zh-TW", "ja", "pt-BR", "es", "fr", "de"];

const DELEGATED_ERRORS: [&str; 4] = ["Spec", "Cert", "Acme", "Instance"];

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Files(#[from] files::Error),
    #[error("locale load failed: {0}")]
    Load(String),
    #[error("Rust parse failed at {}: {source}", path.display())]
    Parse { path: PathBuf, source: syn::Error },
}

pub(crate) struct Report {
    pub(crate) violations: Vec<diagnostic::Violation>,
}

struct Reference {
    path: PathBuf,
    line: usize,
    key: String,
}

pub(crate) fn scan(root: &Path) -> Result<Report, Error> {
    let cdelivery = root.join("cdelivery");
    let locale_dir = cdelivery.join("locales");
    let mut translations = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut violations = Vec::new();
    for namespace in ["ui", "validation"]
        .into_iter()
        .chain(ENUM_LOCALES.map(|(_, _, namespace)| namespace))
    {
        let path = locale_dir.join(format!("{namespace}.toml"));
        let source = files::read(&path)?;
        let locales = rust_i18n_support::try_load_locales(
            &locale_dir.to_string_lossy(),
            |candidate| Path::new(candidate).file_name() != path.file_name(),
            true,
        )
        .map_err(Error::Load)?;
        let finding = |key: &str, text: String| {
            let header = format!("[{key}]");
            diagnostic::Violation {
                path: path.clone(),
                line: source
                    .lines()
                    .position(|line| line.trim() == header)
                    .map_or(1, |index| index + 1),
                text,
            }
        };
        let keys: BTreeSet<_> = locales
            .values()
            .flat_map(|values| values.keys().cloned())
            .collect();
        let empty = BTreeMap::new();
        let values = LANGUAGES.map(|language| locales.get(language).unwrap_or(&empty));
        let prefix = format!("{namespace}.");
        for key in &keys {
            if !key.starts_with(&prefix) {
                violations.push(finding(
                    key,
                    format!("{key} must belong to namespace {namespace}"),
                ));
            }
            if let Some(text) = mismatch(
                key,
                values.map(|values| values.get(key).map(String::as_str)),
            ) {
                violations.push(finding(key, text));
            }
        }
        if let Some(expected) = expected_keys(root, namespace)? {
            for key in expected.difference(&keys) {
                violations.push(finding(key, format!("{key} missing translation key")));
            }
            for key in keys.difference(&expected) {
                violations.push(finding(
                    key,
                    format!("{key} has no corresponding enum variant"),
                ));
            }
        }
        for (language, values) in locales {
            translations.entry(language).or_default().extend(values);
        }
    }
    let empty = BTreeMap::new();
    let translations = LANGUAGES.map(|language| translations.get(language).unwrap_or(&empty));
    let mut references = rust_references(&cdelivery.join("src"))?;
    for path in files::html(&cdelivery.join("templates"))? {
        references.extend(template_references(&path, &files::read(&path)?));
    }
    for reference in references {
        let missing = missing_languages(
            translations.map(|values| values.get(&reference.key).map(String::as_str)),
        );
        if !missing.is_empty() {
            violations.push(diagnostic::Violation {
                path: reference.path,
                line: reference.line,
                text: format!("{} missing {}", reference.key, missing.join(", ")),
            });
        }
    }
    violations.sort_by(|a, b| (&a.path, a.line, &a.text).cmp(&(&b.path, b.line, &b.text)));
    Ok(Report { violations })
}

fn expected_keys(root: &Path, namespace: &str) -> Result<Option<BTreeSet<String>>, Error> {
    let Some(&(source_path, enum_name, _)) =
        ENUM_LOCALES.iter().find(|&&(_, _, name)| name == namespace)
    else {
        return Ok(None);
    };
    let path = root.join(source_path);
    let source = files::read(&path)?;
    let file = syn::parse_file(&source).map_err(|source| Error::Parse { path, source })?;
    let item = file.items.iter().find_map(|item| match item {
        syn::Item::Enum(item) if item.ident == enum_name => Some(item),
        _ => None,
    });
    let Some(item) = item.filter(|item| !item.variants.is_empty()) else {
        return Err(Error::Load(format!("{enum_name} variants were not found")));
    };
    Ok(Some(
        item.variants
            .iter()
            .filter_map(|variant| {
                let name = variant.ident.to_string();
                (enum_name != "ErrorKind" || !DELEGATED_ERRORS.contains(&name.as_str()))
                    .then(|| format!("{namespace}.{name}"))
            })
            .collect(),
    ))
}

fn placeholders(value: &str) -> BTreeSet<String> {
    let mut placeholders = BTreeSet::new();
    let mut rest = value;
    while let Some((_, tail)) = rest.split_once("%{") {
        let Some((name, tail)) = tail.split_once('}') else {
            break;
        };
        placeholders.insert(name.to_owned());
        rest = tail;
    }
    placeholders
}

fn mismatch(key: &str, values: [Option<&str>; LANGUAGES.len()]) -> Option<String> {
    let missing = missing_languages(values);
    if !missing.is_empty() {
        return Some(format!("{key} missing {}", missing.join(", ")));
    }
    let expected = placeholders(values[0]?);
    LANGUAGES
        .into_iter()
        .zip(values)
        .skip(1)
        .find_map(|(language, value)| {
            let actual = placeholders(value?);
            (expected != actual).then(|| {
                format!(
                    "{key} placeholder mismatch: en [{}], {language} [{}]",
                    expected.iter().cloned().collect::<Vec<_>>().join(", "),
                    actual.into_iter().collect::<Vec<_>>().join(", ")
                )
            })
        })
}

fn missing_languages(values: [Option<&str>; LANGUAGES.len()]) -> Vec<&'static str> {
    LANGUAGES
        .into_iter()
        .zip(values)
        .filter_map(|(language, value)| {
            value
                .is_none_or(|value| value.trim().is_empty())
                .then_some(language)
        })
        .collect()
}

fn rust_references(root: &Path) -> Result<Vec<Reference>, Error> {
    let mut references = Vec::new();
    for path in files::rust(root, &[])? {
        let source = files::read(&path)?;
        let file = syn::parse_file(&source).map_err(|source| Error::Parse {
            path: path.clone(),
            source,
        })?;
        let mut visitor = RustVisitor {
            path: &path,
            references: &mut references,
        };
        syn::visit::visit_file(&mut visitor, &file);
    }
    Ok(references)
}

struct RustVisitor<'a> {
    path: &'a Path,
    references: &'a mut Vec<Reference>,
}

impl syn::visit::Visit<'_> for RustVisitor<'_> {
    fn visit_lit_str(&mut self, literal: &syn::LitStr) {
        self.reference(literal);
    }

    fn visit_macro(&mut self, item: &syn::Macro) {
        self.tokens(item.tokens.clone());
        syn::visit::visit_macro(self, item);
    }
}

impl RustVisitor<'_> {
    fn reference(&mut self, literal: &syn::LitStr) {
        let key = literal.value();
        if key.starts_with("ui.") || key.starts_with("validation.") {
            self.references.push(Reference {
                path: self.path.to_path_buf(),
                line: literal.span().start().line,
                key,
            });
        }
    }

    fn tokens(&mut self, tokens: proc_macro2::TokenStream) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Literal(literal) => {
                    if let Ok(value) = syn::parse2::<syn::LitStr>(proc_macro2::TokenStream::from(
                        proc_macro2::TokenTree::Literal(literal),
                    )) {
                        self.reference(&value);
                    }
                }
                proc_macro2::TokenTree::Group(group) => self.tokens(group.stream()),
                _ => {}
            }
        }
    }
}

fn template_references(path: &Path, source: &str) -> Vec<Reference> {
    let mut references = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let mut rest = line;
        while let Some((_, tail)) = rest.split_once("tt!(") {
            let Some(tail) = tail.trim_start().strip_prefix('"') else {
                rest = tail;
                continue;
            };
            let Some((key, tail)) = tail.split_once('"') else {
                break;
            };
            if key.starts_with("ui.") {
                references.push(Reference {
                    path: path.to_path_buf(),
                    line: index + 1,
                    key: key.to_owned(),
                });
            }
            rest = tail;
        }
    }
    references
}
