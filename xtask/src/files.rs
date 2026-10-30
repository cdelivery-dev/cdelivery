use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("io error at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

pub(crate) fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn rust(root: &Path, skip: &[&str]) -> Result<Vec<PathBuf>> {
    extension(root, skip, "rs")
}

pub(crate) fn html(root: &Path) -> Result<Vec<PathBuf>> {
    extension(root, &[], "html")
}

fn extension(root: &Path, skip: &[&str], extension: &str) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect(root, skip, extension, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect(dir: &Path, skip: &[&str], extension: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    let io = |source| Error::Io {
        path: dir.to_path_buf(),
        source,
    };
    for entry in fs::read_dir(dir).map_err(io)? {
        let entry = entry.map_err(io)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(io)?;
        if file_type.is_dir() {
            let skip_it = entry.file_name().to_str().is_some_and(|n| {
                matches!(n, "target" | "node_modules" | ".git") || skip.contains(&n)
            });
            if !skip_it {
                collect(&path, skip, extension, out)?;
            }
        } else if file_type.is_file() && path.extension().is_some_and(|e| e == extension) {
            out.push(path);
        }
    }
    Ok(())
}
