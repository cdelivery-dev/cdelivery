use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct Violation {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) text: String,
}

pub(crate) fn print(root: &Path, violations: &[Violation]) {
    for violation in violations {
        let path = violation.path.strip_prefix(root).unwrap_or(&violation.path);
        println!("{}:{}: {}", path.display(), violation.line, violation.text);
    }
}
