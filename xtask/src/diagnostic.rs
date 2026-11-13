use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct Violation {
    pub(crate) path: PathBuf,
    pub(crate) line: usize,
    pub(crate) text: String,
}

pub(crate) fn print(root: &Path, violations: &[Violation]) {
    for violation in violations {
        let path = violation.path.trim_prefix(root);
        println!("{}:{}: {}", path.display(), violation.line, violation.text);
    }
}
