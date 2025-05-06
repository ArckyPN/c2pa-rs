use std::path::{Path, PathBuf};

pub fn to_signed_path<P1, P2>(path: P1, dir: P2) -> Option<PathBuf>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let file_name = path.as_ref().file_name()?;

    let path = dir.as_ref().join(file_name);

    if path.exists() {
        Some(path)
    } else {
        None
    }
}
