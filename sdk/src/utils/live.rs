use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::{Error, Result};

pub fn signed_output<P>(file: P, output: P) -> Result<Option<PathBuf>>
where
    P: AsRef<Path>,
{
    let file_name = file
        .as_ref()
        .file_name()
        .ok_or(Error::BadParam("file name missing".to_string()))?
        .to_str()
        .ok_or(Error::BadParam("invalid file name".to_string()))?;

    let dir = output
        .as_ref()
        .parent()
        .ok_or(Error::BadParam("output has no parent".to_string()))?;

    let signed_output = dir.join(file_name);

    if signed_output.exists() {
        Ok(Some(signed_output))
    } else {
        Ok(None)
    }
}

pub fn replace_c2pa_box<W>(file: &mut W, buf: &[u8], offset: Option<u64>) -> Result<()>
where
    W: Read + Write + Seek,
{
    let start = match offset {
        Some(o) => o,
        None => unimplemented!("# TODO find the start of the uuid box"),
    };

    file.seek(SeekFrom::Start(start))?;

    // read the size of the current uuid box
    let mut size = [0; 4];
    file.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as u64;

    // buffer every after the uuid box
    file.seek(SeekFrom::Start(start + size))?;
    let mut remainder = Vec::new();
    file.read_to_end(&mut remainder)?;

    // write the new uuid box over the current one
    file.seek(SeekFrom::Start(start))?;
    file.write_all(buf)?;

    // insert the buffered remainder
    file.write_all(&remainder)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir_all, remove_dir_all, remove_file, File, OpenOptions};

    use super::*;

    #[test]
    fn signed_output_test() {
        // test file paths
        let Ok(file) = "/tmp/original/fragment_100.m4s".parse::<PathBuf>();
        let Ok(output) = "/tmp/signed/init.m4s".parse();
        let Ok(actual_output) = "/tmp/signed/fragment_100.m4s".parse::<PathBuf>();

        // create directories
        let Ok(_) = create_dir_all("/tmp/original") else {
            unreachable!()
        };
        let Ok(_) = create_dir_all("/tmp/signed") else {
            unreachable!()
        };

        // create original file and assert it exists
        let Ok(_) = File::create(&file) else {
            unreachable!()
        };
        assert!(file.exists());

        // output equivalent file should not exist yet
        let Ok(not_exist) = signed_output(&file, &output) else {
            unreachable!()
        };
        assert!(not_exist.is_none());

        // create output file and assert it exists
        let Ok(_) = File::create(&actual_output) else {
            unreachable!()
        };
        assert!(actual_output.exists());

        // now it should exist
        let Ok(exists) = signed_output(file, output) else {
            unreachable!()
        };
        let Some(exists) = exists else {
            unreachable!("it should exist now")
        };
        assert_eq!(exists, actual_output);

        // clean up
        let Ok(_) = remove_dir_all("/tmp/original") else {
            unreachable!()
        };
        let Ok(_) = remove_dir_all("/tmp/signed") else {
            unreachable!()
        };
    }

    #[test]
    fn replace_c2pa_box_test() {
        let path = "/tmp/c2pa_box_rest.txt";
        let Ok(mut file) = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
        else {
            unreachable!()
        };

        let data = [
            30u32.to_be_bytes().to_vec(),
            b"ftyp".to_vec(),
            b"some kind of ftyp data".to_vec(),
            25u32.to_be_bytes().to_vec(),
            b"uuid".to_vec(),
            b"more kind of data".to_vec(),
            17u32.to_be_bytes().to_vec(),
            b"moov".to_vec(),
            b"some data".to_vec(),
            17u32.to_be_bytes().to_vec(),
            b"mdat".to_vec(),
            b"this data".to_vec(),
        ]
        .concat();

        let new_uuid_data = [
            57u32.to_be_bytes().to_vec(),
            b"uuid".to_vec(),
            b"this is the new uuid data with a different length".to_vec(),
        ]
        .concat();

        let expected = [
            30u32.to_be_bytes().to_vec(),
            b"ftyp".to_vec(),
            b"some kind of ftyp data".to_vec(),
            new_uuid_data.clone(),
            17u32.to_be_bytes().to_vec(),
            b"moov".to_vec(),
            b"some data".to_vec(),
            17u32.to_be_bytes().to_vec(),
            b"mdat".to_vec(),
            b"this data".to_vec(),
        ]
        .concat();

        let Ok(_) = file.write(&data) else {
            unreachable!()
        };

        let Ok(_) = replace_c2pa_box(&mut file, &new_uuid_data, Some(30)) else {
            unreachable!()
        };

        let Ok(_) = file.rewind() else { unreachable!() };

        let mut actual = Vec::new();
        let Ok(_) = file.read_to_end(&mut actual) else {
            unreachable!()
        };

        assert_eq!(actual, expected);

        let Ok(_) = remove_file(path) else {
            unreachable!()
        };
    }
}
