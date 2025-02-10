use std::{
    fs::{read_dir, remove_dir_all},
    path::Path,
};

use anyhow::{Context, Result};
use rocket::{
    data::ByteUnit,
    tokio::{
        fs::{create_dir_all, File},
        io::{AsyncReadExt, AsyncWriteExt},
    },
    Data,
};

const MAX_CHUNK_SIZE: usize = u16::MAX as usize;

/// cleans up all media created during runtime
///
/// deletes all subdirectories found in `dir`
pub fn clear_media<P>(dir: P) -> Result<()>
where
    P: AsRef<Path>,
{
    for entry in read_dir(dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                log::warn!("unable to read: {err}");
                continue;
            }
        };

        if !entry.metadata()?.is_dir() {
            continue;
        }

        remove_dir_all(entry.path())?;
    }
    Ok(())
}

/// reads the request body, copies it to local disc and returns it as buffer
pub(crate) async fn process_request_body<P>(body: Data<'_>, path: P) -> Result<Vec<u8>>
where
    P: AsRef<Path>,
{
    let mut file = create_file(path).await?;

    let mut body = body.open(ByteUnit::max_value());
    let mut buf = Vec::new();
    loop {
        let mut chunk = vec![0; MAX_CHUNK_SIZE];
        let read = body.read(&mut chunk).await?;
        if read == 0 {
            // EOS
            break;
        }

        let chunk = &chunk[..read];
        buf.extend_from_slice(chunk);
        file.write_all(chunk).await?;
    }

    Ok(buf)
}

/// creates the file at `path`
///
/// creates the path to file, if it doesn't exist
async fn create_file<P>(path: P) -> Result<File>
where
    P: AsRef<Path>,
{
    create_path_to_file(&path).await?;

    Ok(File::create(path).await?)
}

/// creates the path to the parent of `path`
async fn create_path_to_file<P>(path: P) -> Result<()>
where
    P: AsRef<Path>,
{
    let dir = path
        .as_ref()
        .parent()
        .context(format!("path {:?} has no parent", path.as_ref()))?;

    Ok(create_dir_all(dir).await?)
}

/// checks wether `uri` is a fragment path
///
/// path extension equal `m4s`
pub(crate) fn is_fragment<P>(uri: P) -> bool
where
    P: AsRef<Path>,
{
    let Some(ext) = uri.as_ref().extension() else {
        return false;
    };
    match ext.to_str() {
        Some(ext) => ext == "m4s",
        None => false,
    }
}

/// checks wether `uri` is a init fragment path
///
/// file name contains `init`
pub(crate) fn is_init<P>(uri: P) -> bool
where
    P: AsRef<Path>,
{
    match uri.as_ref().file_name() {
        Some(s) => match s.to_str() {
            Some(s) => s.contains("init"),
            None => false,
        },
        None => false,
    }
}
