#![expect(clippy::print_stdout, clippy::print_stderr)]

mod comments;
mod diagnostic;
mod env;
#[path = "../../tests/environment.rs"]
mod environment;
mod files;
mod imports;
mod loc;
mod locale;
#[path = "../../tests/process.rs"]
mod process;
mod ui;

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let args: Vec<_> = std::env::args().skip(1).collect();
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    match run(&root, &args) {
        Ok(code) => code,
        Err(error) => {
            let task = args.first().copied().unwrap_or("xtask");
            eprintln!("{task} failed: {error:?}");
            ExitCode::FAILURE
        }
    }
}

fn run(root: &Path, args: &[&str]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let success = match args {
        ["loc"] => {
            let report = loc::scan(root)?;
            let row = |name: &str, count: loc::Count| {
                println!("{name:<16} {:>10} {:>10}", count.code, count.tests);
            };
            println!("{:<16} {:>10} {:>10}", "crate", "code", "tests");
            for (name, count) in &report.crates {
                row(name, *count);
            }
            row("TOTAL", report.total);
            if report.parse_failures > 0 {
                println!(
                    "note: {} file(s) failed to parse, counted as code",
                    report.parse_failures
                );
            }
            true
        }
        ["imports"] => {
            let report = imports::scan(root)?;
            diagnostic::print(root, &report.violations);
            for path in &report.parse_failures {
                let path = path.strip_prefix(root).unwrap_or(path);
                println!("{}: failed to parse", path.display());
            }
            let clean = report.violations.is_empty() && report.parse_failures.is_empty();
            if clean {
                println!("imports: clean");
            } else {
                println!(
                    "imports: {} violation(s), {} parse failure(s)",
                    report.violations.len(),
                    report.parse_failures.len()
                );
            }
            clean
        }
        ["comments"] => {
            let report = comments::scan(root)?;
            diagnostic::print(root, &report.violations);
            for path in &report.lex_failures {
                let path = path.strip_prefix(root).unwrap_or(path);
                println!("{}: failed to lex", path.display());
            }
            let clean = report.violations.is_empty() && report.lex_failures.is_empty();
            if clean {
                println!("comments: none");
            } else {
                println!(
                    "comments: {} found, {} lex failure(s)",
                    report.violations.len(),
                    report.lex_failures.len()
                );
            }
            clean
        }
        [task @ ("locale" | "ui")] => {
            let violations = if *task == "locale" {
                locale::scan(root)?.violations
            } else {
                ui::scan(root)?.violations
            };
            diagnostic::print(root, &violations);
            if violations.is_empty() {
                let status = if *task == "locale" {
                    "complete"
                } else {
                    "clean"
                };
                println!("{task}: {status}");
            } else {
                println!("{task}: {} violation(s)", violations.len());
            }
            violations.is_empty()
        }
        ["env", args @ ..] => {
            env::run(root, args)?;
            true
        }
        [] | ["help" | "--help"] => {
            usage();
            true
        }
        _ => {
            eprintln!("invalid arguments: {}", args.join(" "));
            usage();
            false
        }
    };
    Ok(ExitCode::from(u8::from(!success)))
}

fn usage() {
    println!("usage: cargo xtask <task>");
    println!("  loc       count production/test lines without a size limit");
    println!("  comments  check TODO/FIXME/BUG/FIX comments and contract docs");
    println!("  imports   check layer imports");
    println!("  locale    check translation keys, languages and placeholders");
    println!("  ui        check localized JavaScript and required assets");
    println!("  env       up <single|cluster|storage> | down | clean");
    println!("  help      show this help (--help also works)");
}
