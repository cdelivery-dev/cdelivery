use std::collections::BTreeMap;
use std::path::Path;

use crate::{diagnostic, files};

pub(crate) struct Report {
    pub(crate) violations: Vec<diagnostic::Violation>,
}

pub(crate) fn scan(root: &Path) -> files::Result<Report> {
    let cdelivery = root.join("cdelivery");
    let mut violations = template_violations(&cdelivery.join("templates"))?;
    violations.extend(asset_violations(&cdelivery)?);
    violations.sort_by(|a, b| (&a.path, a.line, &a.text).cmp(&(&b.path, b.line, &b.text)));
    Ok(Report { violations })
}

fn template_violations(templates: &Path) -> files::Result<Vec<diagnostic::Violation>> {
    let mut violations = Vec::new();
    for path in files::html(templates)? {
        let source = files::read(&path)?;
        for (name, text) in [
            ("confirm", "raw localized confirm string"),
            ("alert", "raw localized alert string"),
        ] {
            for offset in raw_localized_calls(&source, name) {
                violations.push(diagnostic::Violation {
                    path: path.clone(),
                    line: line_at(&source, offset),
                    text: format!("{text} is forbidden"),
                });
            }
        }
        for offset in raw_localized_x_texts(&source) {
            violations.push(diagnostic::Violation {
                path: path.clone(),
                line: line_at(&source, offset),
                text: "raw localized x-text string is forbidden".to_owned(),
            });
        }
    }
    Ok(violations)
}

fn asset_violations(cdelivery: &Path) -> files::Result<Vec<diagnostic::Violation>> {
    let base = cdelivery.join("templates/base.html");
    let source = files::read(&base)?;
    let mut violations = Vec::new();
    for (name, label) in [
        ("STYLE_PATH.as_str()", "main CSS path"),
        ("SHARED_PATH.as_str()", "shared JS path"),
        ("FAVICON_LIGHT_PATH.as_str()", "light favicon path"),
        ("FAVICON_DARK_PATH.as_str()", "dark favicon path"),
        ("ALPINE_PATH", "Alpine path"),
    ] {
        violations.extend(expected_reference(
            &base,
            &source,
            &template_asset_path(name),
            label,
        ));
    }
    let shared_path = template_asset_path("SHARED_PATH.as_str()");
    let alpine_path = template_asset_path("ALPINE_PATH");
    for (path, label) in [(&shared_path, "shared JS"), (&alpine_path, "Alpine")] {
        let valid = script_attributes(&source, path).is_some_and(|attributes| {
            attributes.contains_key("defer")
                && !attributes.contains_key("async")
                && !attributes.contains_key("nomodule")
                && attributes.get("type").is_none_or(|kind| {
                    ["", "text/javascript", "application/javascript"]
                        .iter()
                        .any(|expected| kind.eq_ignore_ascii_case(expected))
                })
        });
        if !valid {
            violations.push(diagnostic::Violation {
                path: base.clone(),
                line: source
                    .find(path)
                    .map_or(1, |offset| line_at(&source, offset)),
                text: format!("{label} must use a classic defer script without async or nomodule"),
            });
        }
    }
    if let (Some(shared), Some(alpine)) = (source.find(&shared_path), source.find(&alpine_path))
        && shared > alpine
    {
        violations.push(diagnostic::Violation {
            path: base,
            line: line_at(&source, shared),
            text: "shared JS must load before Alpine".to_owned(),
        });
    }
    Ok(violations)
}

fn script_attributes<'a>(source: &'a str, path: &str) -> Option<BTreeMap<String, &'a str>> {
    for mut tail in source.split("<script").skip(1) {
        if !tail.starts_with(|ch: char| ch.is_ascii_whitespace() || ch == '>') {
            continue;
        }
        let mut attributes = BTreeMap::new();
        loop {
            tail = tail.trim_start();
            if tail.starts_with('>') {
                break;
            }
            let end = tail.find(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '=' | '>'))?;
            let name = tail.get(..end)?;
            if name.is_empty() {
                return None;
            }
            tail = tail.get(end..)?.trim_start();
            let value = if let Some(rest) = tail.strip_prefix('=') {
                let rest = rest.trim_start();
                if let Some(&quote @ (b'\'' | b'"')) = rest.as_bytes().first() {
                    let rest = rest.get(1..)?;
                    let value = quoted_value(rest, quote)?;
                    tail = rest.get(value.len() + 1..)?;
                    value
                } else {
                    let end = rest.find(|ch: char| ch.is_ascii_whitespace() || ch == '>')?;
                    tail = rest.get(end..)?;
                    rest.get(..end)?
                }
            } else {
                ""
            };
            if attributes
                .insert(name.to_ascii_lowercase(), value)
                .is_some()
            {
                return None;
            }
        }
        if attributes.get("src") == Some(&path) {
            return tail.contains("</script>").then_some(attributes);
        }
    }
    None
}

fn template_asset_path(name: &str) -> String {
    format!("{{{{ crate::ui::asset::{name} }}}}")
}

fn expected_reference(
    path: &Path,
    source: &str,
    expected: &str,
    label: &str,
) -> Vec<diagnostic::Violation> {
    let count = source.matches(expected).count();
    if count == 1 {
        Vec::new()
    } else {
        vec![diagnostic::Violation {
            path: path.to_path_buf(),
            line: 1,
            text: format!("expected one {label} {expected}, found {count}"),
        }]
    }
}

fn raw_localized_calls(source: &str, name: &str) -> Vec<usize> {
    source
        .match_indices(name)
        .filter_map(|(offset, _)| {
            let before = source.get(..offset)?;
            if before.chars().next_back().is_some_and(is_identifier_char) {
                return None;
            }
            let tail = source.get(offset + name.len()..)?.trim_start();
            let tail = tail.strip_prefix('(')?;
            let script = active_script(before);
            let expression = if script {
                tail.split_once('\n').map_or(tail, |(line, _)| line)
            } else {
                attribute_expression(before, tail)?
            };
            raw_localized_expression(expression, script).then_some(offset)
        })
        .collect()
}

fn raw_localized_x_texts(source: &str) -> Vec<usize> {
    source
        .match_indices("x-text")
        .filter_map(|(offset, _)| {
            if source
                .get(..offset)?
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_ascii_whitespace() && ch != '<')
            {
                return None;
            }
            let tail = source.get(offset + "x-text".len()..)?.trim_start();
            let value = tail.strip_prefix('=')?.trim_start();
            let quote = *value.as_bytes().first()?;
            if !matches!(quote, b'\'' | b'"') {
                return None;
            }
            let expression = quoted_value(value.get(1..)?, quote)?;
            raw_localized_expression(expression, false).then_some(offset)
        })
        .collect()
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$')
}

fn active_script(before: &str) -> bool {
    let opening = before.rfind("<script");
    let closing = before.rfind("</script>");
    opening.is_some_and(|start| closing.is_none_or(|end| start > end))
}

fn attribute_expression<'a>(before: &str, tail: &'a str) -> Option<&'a str> {
    let line = before.rsplit_once('\n').map_or(before, |(_, line)| line);
    let start = [line.rfind('@'), line.rfind("x-")]
        .into_iter()
        .flatten()
        .max()?;
    let attribute = line.get(start..)?;
    let value = attribute.get(attribute.find('=')? + 1..)?.trim_start();
    let quote = *value.as_bytes().first()?;
    matches!(quote, b'\'' | b'"').then(|| quoted_value(tail, quote))?
}

fn quoted_value(source: &str, quote: u8) -> Option<&str> {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if let Some(end) = template_block_end(source, cursor) {
            cursor = end;
        } else if *bytes.get(cursor)? == quote {
            return source.get(..cursor);
        } else {
            cursor += 1;
        }
    }
    None
}

fn raw_localized_expression(source: &str, script: bool) -> bool {
    let mut cursor = 0;
    while let Some(relative) = source.get(cursor..).and_then(|tail| tail.find("{{")) {
        let start = cursor + relative;
        let Some(end) = template_block_end(source, start) else {
            return source
                .get(start..)
                .is_some_and(|block| block.contains("tt!("));
        };
        let Some(block) = source.get(start..end) else {
            return false;
        };
        if block.contains("tt!(") && (!script || !block.contains("|json|safe")) {
            return true;
        }
        cursor = end;
    }
    false
}

fn template_block_end(source: &str, start: usize) -> Option<usize> {
    for (opening, closing) in [("{{", "}}"), ("{%", "%}")] {
        if source.get(start..)?.starts_with(opening) {
            return source
                .get(start + opening.len()..)?
                .find(closing)
                .map(|end| start + opening.len() + end + closing.len());
        }
    }
    None
}

fn line_at(source: &str, offset: usize) -> usize {
    source
        .bytes()
        .take(offset)
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}
