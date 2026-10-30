use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{Delimiter, Group, LineColumn, Span, TokenStream, TokenTree};

use crate::{diagnostic, files};

pub(crate) struct Report {
    pub(crate) violations: Vec<diagnostic::Violation>,
    pub(crate) lex_failures: Vec<PathBuf>,
}

pub(crate) fn scan(root: &Path) -> files::Result<Report> {
    let paths = files::rust(root, &[])?;
    let operation = root.join("cdelivery/src/operation");

    let mut violations = Vec::new();
    let mut lex_failures = Vec::new();
    for path in &paths {
        let source = files::read(path)?;
        let contract = path
            .strip_prefix(&operation)
            .is_ok_and(|rest| rest.iter().any(|part| part == "typ" || part == "typ.rs"));
        match check_source(&source, contract) {
            Some(findings) => {
                violations.extend(findings.into_iter().map(|f| diagnostic::Violation {
                    path: path.clone(),
                    line: f.line,
                    text: f.text,
                }));
            }
            None => lex_failures.push(path.clone()),
        }
    }
    lex_failures.sort();
    Ok(Report {
        violations,
        lex_failures,
    })
}

pub(crate) fn code_lines(source: &str) -> Option<BTreeSet<usize>> {
    let Lexed { starts, spans, .. } = lex(source)?;
    let mut lines = BTreeSet::new();
    for (start, end) in spans {
        lines.extend(line_of(&starts, start)..=line_of(&starts, end.saturating_sub(1)));
    }
    Some(lines)
}

struct Lexed {
    chars: Vec<char>,
    starts: Vec<usize>,
    spans: Vec<(usize, usize)>,
}

fn lex(source: &str) -> Option<Lexed> {
    let stream = TokenStream::from_str(source).ok()?;
    let chars: Vec<char> = source.chars().collect();
    let starts = line_starts(&chars);
    let mut spans = Vec::new();
    collect_spans(stream, &starts, &mut spans);
    Some(Lexed {
        chars,
        starts,
        spans,
    })
}

struct Finding {
    line: usize,
    text: String,
}

fn check_source(source: &str, contract: bool) -> Option<Vec<Finding>> {
    let Lexed {
        chars,
        starts,
        mut spans,
    } = lex(source)?;
    spans.sort_unstable();

    let mut findings = Vec::new();
    let mut cursor = 0;
    for &(start, end) in &spans {
        if start > cursor {
            extract_comments(&chars, cursor, start, &starts, &mut findings);
        }
        cursor = cursor.max(end);
    }
    if cursor < chars.len() {
        extract_comments(&chars, cursor, chars.len(), &starts, &mut findings);
    }

    findings.extend(doc_findings(source));
    findings.retain(|f| !(allowed(&f.text) || contract && f.text.starts_with("/// ")));
    findings.sort_by_key(|f| f.line);
    Some(findings)
}

const MARKERS: [&str; 4] = ["TODO", "FIXME", "BUG", "FIX"];

fn allowed(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("// ") else {
        return false;
    };
    MARKERS.iter().any(|kw| {
        rest.strip_prefix(kw)
            .and_then(|r| r.strip_prefix(": "))
            .is_some_and(|body| !body.trim().is_empty())
    })
}

fn doc_findings(source: &str) -> Vec<Finding> {
    let mut lines = Vec::new();
    if let Ok(stream) = TokenStream::from_str(source) {
        collect_doc_lines(stream, &mut lines);
    }
    lines.sort_unstable();
    lines.dedup();
    lines
        .into_iter()
        .map(|line| Finding {
            line,
            text: source
                .lines()
                .nth(line.saturating_sub(1))
                .map(str::trim)
                .unwrap_or_default()
                .to_owned(),
        })
        .collect()
}

fn collect_doc_lines(stream: TokenStream, out: &mut Vec<usize>) {
    let mut tokens = stream.into_iter().peekable();
    while let Some(tt) = tokens.next() {
        match tt {
            TokenTree::Group(group) => collect_doc_lines(group.stream(), out),
            TokenTree::Punct(punct) if punct.as_char() == '#' => {
                if matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '!') {
                    tokens.next();
                }
                if matches!(tokens.peek(), Some(TokenTree::Group(g)) if is_doc_attr(g)) {
                    tokens.next();
                    out.push(punct.span().start().line);
                }
            }
            _ => {}
        }
    }
}

fn is_doc_attr(group: &Group) -> bool {
    if group.delimiter() != Delimiter::Bracket {
        return false;
    }
    let mut inner = group.stream().into_iter();
    matches!(inner.next(), Some(TokenTree::Ident(ident)) if ident == "doc")
        && matches!(inner.next(), Some(TokenTree::Punct(p)) if p.as_char() == '=')
}

fn collect_spans(stream: TokenStream, starts: &[usize], out: &mut Vec<(usize, usize)>) {
    for tt in stream {
        if let TokenTree::Group(group) = &tt {
            push_span(group.span_open(), starts, out);
            collect_spans(group.stream(), starts, out);
            push_span(group.span_close(), starts, out);
        } else {
            push_span(tt.span(), starts, out);
        }
    }
}

fn push_span(span: Span, starts: &[usize], out: &mut Vec<(usize, usize)>) {
    out.push((
        offset_of(starts, span.start()),
        offset_of(starts, span.end()),
    ));
}

fn offset_of(starts: &[usize], lc: LineColumn) -> usize {
    starts.get(lc.line.saturating_sub(1)).copied().unwrap_or(0) + lc.column
}

fn line_starts(chars: &[char]) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn line_of(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&s| s <= offset)
}

fn extract_comments(
    chars: &[char],
    from: usize,
    to: usize,
    starts: &[usize],
    out: &mut Vec<Finding>,
) {
    let mut i = from;
    while i < to {
        let here = chars.get(i).copied();
        let next = chars.get(i + 1).copied();
        if here == Some('/') && next == Some('/') {
            let mut j = i;
            while j < to && chars.get(j) != Some(&'\n') {
                j += 1;
            }
            out.push(make_finding(chars, starts, i, j));
            i = j;
        } else if here == Some('/') && next == Some('*') {
            let mut depth = 0;
            let mut j = i;
            while j < to {
                let a = chars.get(j).copied();
                let b = chars.get(j + 1).copied();
                if a == Some('/') && b == Some('*') {
                    depth += 1;
                    j += 2;
                } else if a == Some('*') && b == Some('/') {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            out.push(make_finding(chars, starts, i, j));
            i = j;
        } else {
            i += 1;
        }
    }
}

fn make_finding(chars: &[char], starts: &[usize], start: usize, end: usize) -> Finding {
    let text: String = chars
        .get(start..end)
        .map(|seg| seg.iter().collect())
        .unwrap_or_default();
    Finding {
        line: line_of(starts, start),
        text: text.lines().next().unwrap_or_default().trim().to_owned(),
    }
}
