use std::{
    fs::{read_dir, remove_dir_all},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use bytes::{Buf, Bytes};
use rocket::{
    data::ByteUnit,
    tokio::{
        fs::{create_dir_all, File},
        io::{AsyncReadExt, AsyncWriteExt},
    },
    Data,
};

const MAX_CHUNK_SIZE: usize = u16::MAX as usize;

#[macro_export]
macro_rules! log_err {
    ($fn:expr, $name:expr) => {
        $fn.map_err(|err| {
            log::error!("{}: {err}", $name);
            rocket::http::Status::InternalServerError
        })
    };
    ($fn:expr, $name:expr, $err:expr) => {
        $fn.map_err(|err| {
            log::error!("{}: {err}", $name);
            $err
        })
    };
}

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

pub(crate) fn extract_c2pa_box<P>(path: P) -> Result<Vec<u8>>
where
    P: AsRef<Path>,
{
    let buf = std::fs::read(&path)?;
    let mut buf = Bytes::copy_from_slice(&buf);
    let mut c2pa = None;

    loop {
        let size = buf.get_u32();
        let name = buf.copy_to_bytes(4);

        let (size, hdr) = match size {
            1 => (buf.get_u64(), 8),
            _ => (size as u64, 4),
        };

        let payload_size = size as usize - hdr - 4;

        if *name == *b"uuid" {
            // FIXME ideally handle large size as well but unlikely to happen
            let mut size = (size as u32).to_be_bytes().to_vec();
            let mut name = name.to_vec();
            let mut payload = buf.copy_to_bytes(payload_size).to_vec();

            size.append(&mut name);
            size.append(&mut payload);
            c2pa.replace(size);
            break;
        }

        buf.advance(payload_size);
    }

    if let Some(c2pa) = c2pa {
        Ok(c2pa)
    } else {
        bail!("missing c2pa box in {:?}", path.as_ref())
    }
}

pub(crate) fn find_init<P>(dir: P) -> Result<PathBuf>
where
    P: AsRef<Path>,
{
    for entry in dir.as_ref().read_dir()? {
        let path = entry?.path();
        if is_init(&path) {
            return Ok(path);
        }
    }
    bail!("could not find init")
}

#[allow(dead_code)]
pub(crate) fn replace_uuid_content<P>(path: P, new_content: &[u8]) -> Result<Vec<u8>>
where
    P: AsRef<Path>,
{
    let buf = std::fs::read(&path)?;
    let mut buf = Bytes::copy_from_slice(&buf);

    let mut vec = Vec::new();
    while buf.has_remaining() {
        let size = buf.get_u32();
        let name = buf.copy_to_bytes(4);

        if size == 1 {
            unimplemented!("large boxes")
        }

        let payload_size = size as usize - 8;

        if *name == *b"uuid" {
            let new_len = new_content.len() as u32 + 8;

            vec.append(&mut new_len.to_be_bytes().to_vec());
            vec.append(&mut name.into());
            vec.append(&mut new_content.to_vec());

            buf.advance(payload_size);
        } else {
            vec.append(&mut size.to_be_bytes().to_vec());
            vec.append(&mut name.into());
            vec.append(&mut buf.copy_to_bytes(payload_size).into());
        }
    }

    Ok(vec)
}

#[allow(dead_code)]
pub(crate) fn mpd_num_reps(mpd: &dash_mpd::MPD) -> usize {
    let mut num = 0;

    for period in &mpd.periods {
        for adaptation in &period.adaptations {
            num += adaptation.representations.len();
        }
    }

    num
}

#[cfg(test)]
mod tests {
    #[test]
    /// test for only normal box sizes
    fn replace_uuid_content_normal() {
        let path = "/tmp/c2pa_data";
        let og = [
            28_u32.to_be_bytes().to_vec(),
            b"ftyp".to_vec(),
            b"this is some content".to_vec(),
            33_u32.to_be_bytes().to_vec(),
            b"uuid".to_vec(),
            b"the original uuid content".to_vec(),
            31_u32.to_be_bytes().to_vec(),
            b"mdat".to_vec(),
            b"here we some media data".to_vec(),
        ]
        .concat();

        let exp = [
            28_u32.to_be_bytes().to_vec(),
            b"ftyp".to_vec(),
            b"this is some content".to_vec(),
            56_u32.to_be_bytes().to_vec(),
            b"uuid".to_vec(),
            b"http://localhost:5000/c2pa/bbb/0/source_init.m4s".to_vec(),
            31_u32.to_be_bytes().to_vec(),
            b"mdat".to_vec(),
            b"here we some media data".to_vec(),
        ]
        .concat();

        std::fs::write(path, &og).unwrap();

        let rep = super::replace_uuid_content(
            path,
            "http://localhost:5000/c2pa/bbb/0/source_init.m4s".as_bytes(),
        )
        .unwrap();

        assert_eq!(
            exp, rep,
            "replace uuid box does not work for non large header"
        );

        std::fs::remove_file(path).unwrap();
    }
}
