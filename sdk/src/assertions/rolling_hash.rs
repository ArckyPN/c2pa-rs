use std::{
    io::{Cursor, Read, Seek},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

use crate::{
    assertion::{AssertionBase, AssertionCbor},
    asset_handlers::bmff_io::{bmff_to_jumbf_exclusions, C2PABmffBoxesRollingHash},
    asset_io::CAIRead,
    hash_stream_by_alg,
    hash_utils::{concat_and_hash, verify_stream_by_alg},
    Error, Result,
};

use super::ExclusionsMap;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct RollingHash {
    exclusions: Vec<ExclusionsMap>,

    /// The Hashing Algorithm used.
    #[serde(skip_serializing_if = "Option::is_none")]
    alg: Option<String>,

    /// The hash of this asset (BMFF Init File)
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<ByteBuf>,

    /// The rolling hash result.
    ///
    /// Used during validation for comparison.
    #[serde(skip_serializing_if = "Option::is_none")]
    rolling_hash: Option<ByteBuf>,

    /// The previous rolling hash.
    ///
    /// Used as anchor point, when joining
    /// the live stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_hash: Option<ByteBuf>,

    /// The name of this assertion.
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl RollingHash {
    pub const LABEL: &'static str = crate::assertions::labels::ROLLING_HASH;

    pub fn new(name: &str, alg: &str) -> Self {
        Self {
            exclusions: Vec::new(),
            alg: Some(alg.to_string()),
            hash: None,
            rolling_hash: None,
            previous_hash: None,
            name: Some(name.to_string()),
        }
    }

    pub fn exclusions(&self) -> &[ExclusionsMap] {
        self.exclusions.as_ref()
    }

    pub fn exclusions_mut(&mut self) -> &mut Vec<ExclusionsMap> {
        &mut self.exclusions
    }

    pub fn alg(&self) -> Option<&String> {
        self.alg.as_ref()
    }

    pub fn hash(&self) -> Option<&Vec<u8>> {
        self.hash.as_deref()
    }

    pub fn set_hash(&mut self, hash: Vec<u8>) {
        self.hash = Some(ByteBuf::from(hash));
    }

    pub fn clear_hash(&mut self) {
        self.hash = None;
    }

    pub fn rolling_hash(&self) -> Option<&Vec<u8>> {
        self.rolling_hash.as_deref()
    }

    pub fn set_rolling_hash(&mut self, hash: Vec<u8>) {
        self.rolling_hash = Some(ByteBuf::from(hash));
    }

    pub fn clear_rolling_hash(&mut self) {
        self.rolling_hash = None;
    }

    pub fn previous_hash(&self) -> Option<&Vec<u8>> {
        self.previous_hash.as_deref()
    }

    pub fn set_previous_hash(&mut self, hash: Vec<u8>) {
        self.previous_hash = Some(ByteBuf::from(hash));
    }

    pub fn clear_previous_hash(&mut self) {
        self.previous_hash = None;
    }

    pub fn name(&self) -> Option<&String> {
        self.name.as_ref()
    }

    /// moves the rolling hash to the previous hash
    pub fn shift_rolling_hash(&mut self) {
        self.previous_hash = self.rolling_hash.take();
    }

    /// Generate the hash value for the asset using the range from the RollingHash.
    pub fn gen_hash_from_stream<R>(&mut self, asset_stream: &mut R) -> crate::error::Result<()>
    where
        R: Read + Seek + ?Sized,
    {
        self.hash = Some(ByteBuf::from(self.hash_from_stream(asset_stream)?));
        Ok(())
    }

    /// Generate the asset hash from a file asset using the constructed
    /// start and length values.
    fn hash_from_stream<R>(&mut self, asset_stream: &mut R) -> crate::error::Result<Vec<u8>>
    where
        R: Read + Seek + ?Sized,
    {
        let alg = match self.alg {
            Some(ref a) => a.clone(),
            None => "sha256".to_string(),
        };

        let bmff_exclusions = &self.exclusions;

        // convert BMFF exclusion map to flat exclusion list
        let exclusions = bmff_to_jumbf_exclusions(asset_stream, bmff_exclusions, true)?;

        let hash = hash_stream_by_alg(&alg, asset_stream, Some(exclusions), true)?;

        if hash.is_empty() {
            Err(Error::BadParam("could not generate data hash".to_string()))
        } else {
            Ok(hash)
        }
    }

    #[cfg(feature = "file_io")]
    pub fn update_fragmented_inithash<P>(&mut self, asset_path: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let mut reader = std::fs::File::open(asset_path)?;

        let alg = self.alg().cloned().unwrap_or("sha256".to_string());

        let exclusions = bmff_to_jumbf_exclusions(&mut reader, self.exclusions(), true)?;
        reader.rewind()?;
        let hash = hash_stream_by_alg(&alg, &mut reader, Some(exclusions), true)?;

        self.hash.replace(hash.into());

        Ok(())
    }

    pub fn add_new_fragment<P1, P2, P3>(
        &mut self,
        alg: &str,
        asset_path: P1,
        fragment: P2,
        output_dir: P3,
    ) -> Result<()>
    where
        P1: AsRef<Path>,
        P2: AsRef<Path>,
        P3: AsRef<Path>,
    {
        // create output dir, if it doesn't exist
        if !output_dir.as_ref().exists() {
            std::fs::create_dir_all(&output_dir)?;
        } else if !output_dir.as_ref().is_dir() {
            // make sure it is a directory
            return Err(Error::BadParam("output_dir is not a directory".to_string()));
        }

        // copy fragment to output dir
        let file_name = fragment
            .as_ref()
            .file_name()
            .ok_or(Error::BadParam("invalid fragment path".to_string()))?;
        let fragment_output = output_dir.as_ref().join(file_name);
        std::fs::copy(&fragment, &fragment_output)?;

        // copy init file, if its output doesn't exist
        let file_name = asset_path
            .as_ref()
            .file_name()
            .ok_or(Error::BadParam("invalid fragment path".to_string()))?;
        let init_output = output_dir.as_ref().join(file_name);
        if !init_output.exists() {
            std::fs::copy(&asset_path, &init_output)?;
        }

        let mut reader = std::fs::File::open(&fragment)?;
        let c2pa_boxes = C2PABmffBoxesRollingHash::from_reader(&mut reader)?;
        let box_infos = &c2pa_boxes.box_infos;

        if box_infos.iter().filter(|b| b.path == "moof").count() != 1 {
            return Err(Error::BadParam("expected 1 moof in fragment".to_string()));
        }
        if box_infos.iter().filter(|b| b.path == "mdat").count() != 1 {
            return Err(Error::BadParam("expected 1 mdat in fragment".to_string()));
        }

        // ensure there aren't more than one uuid box
        if c2pa_boxes.rolling_hashes.len() > 1 || c2pa_boxes.bmff_merkle_box_infos.len() > 1 {
            return Err(Error::BadParam(
                "BMFF Fragments shouldn't have more than 1 BmffMerkleMap".to_string(),
            ));
        }

        // build the UUID Box of the Fragment
        // box content is simply the previous rolling hash
        let anchor_data = FragmentRollingHash {
            anchor_point: self.previous_hash.clone(),
        };
        let anchor_data = serde_cbor::to_vec(&anchor_data)
            .map_err(|err| Error::AssertionEncoding(err.to_string()))?;

        let mut uuid_box_data = Vec::with_capacity(anchor_data.len() * 2);
        crate::asset_handlers::bmff_io::write_c2pa_box(
            &mut uuid_box_data,
            &[],
            false,
            &anchor_data,
        )?;

        // insert the UUID Box in the output Fragment
        let mut source = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&fragment)?;
        let mut dest = std::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .open(&fragment_output)?;
        let first_moof = box_infos
            .iter()
            .find(|b| b.path == "moof")
            .ok_or(Error::BadParam("expected 1 moof in fragment".to_string()))?;
        crate::utils::io_utils::insert_data_at(
            &mut source,
            &mut dest,
            first_moof.offset,
            &uuid_box_data,
        )?;

        // create the new rolling hash: hash(previous hash + fragment hash)
        let hash_ranges = bmff_to_jumbf_exclusions(&mut dest, self.exclusions(), true)?;
        let fragment_hash = hash_stream_by_alg(alg, &mut dest, Some(hash_ranges), true)?;

        // prepare required hashes
        let (left, right) = if let Some(prev) = self.previous_hash() {
            // when previous available: previous + fragment
            (prev, Some(fragment_hash.as_slice()))
        } else {
            // otherwise: only fragment
            (&fragment_hash, None)
        };

        // set the actual rolling hash
        self.rolling_hash
            .replace(concat_and_hash(alg, left, right).into());

        // set placeholder for init hash
        self.hash = Some(match alg {
            // placeholder init hash to be filled once manifest is inserted
            "sha256" => ByteBuf::from([0u8; 32].to_vec()),
            "sha384" => ByteBuf::from([0u8; 48].to_vec()),
            "sha512" => ByteBuf::from([0u8; 64].to_vec()),
            _ => return Err(Error::UnsupportedType),
        });

        Ok(())
    }

    #[cfg(feature = "file_io")]
    pub fn verify_hash(&self, asset_path: &std::path::Path, alg: Option<&str>) -> Result<()> {
        let mut data = std::fs::File::open(asset_path)?;
        self.verify_stream_hash(&mut data, alg)
    }

    /// Verifies RollingHash in BMFF content from a single file asset. The following variants are handles:
    /// * A single BMFF asset with only a file hash (the Init File Hash)
    pub fn verify_stream_hash(&self, reader: &mut dyn CAIRead, alg: Option<&str>) -> Result<()> {
        reader.rewind()?;
        // let size = crate::utils::io_utils::stream_len(reader)?;

        let curr_alg = match &self.alg {
            Some(a) => a.clone(),
            None => match alg {
                Some(a) => a.to_owned(),
                None => "sha256".to_string(),
            },
        };

        // convert BMFF exclusion map to flat exclusion list
        let exclusions = bmff_to_jumbf_exclusions(reader, &self.exclusions, true)?;

        // handle file level hashing
        if let Some(hash) = self.hash() {
            if !verify_stream_by_alg(&curr_alg, hash, reader, Some(exclusions.clone()), true) {
                return Err(Error::HashMismatch(
                    "BMFF file level hash mismatch".to_string(),
                ));
            }
        }

        Ok(())
    }

    pub fn verify_in_memory_hash(&self, data: &[u8], alg: Option<&str>) -> Result<()> {
        let mut reader = Cursor::new(data);

        self.verify_stream_hash(&mut reader, alg)
    }

    pub fn verify_stream_segment(
        &self,
        init_stream: &mut dyn CAIRead,
        fragment_stream: &mut dyn CAIRead,
        alg: Option<&str>,
    ) -> Result<()> {
        let curr_alg = match &self.alg {
            Some(a) => a.clone(),
            None => match alg {
                Some(a) => a.to_owned(),
                None => "sha256".to_string(),
            },
        };

        // handle file level hashing
        if self.hash().is_some() {
            self.verify_stream_hash(init_stream, alg)?;
        }

        // validate previous hash with fragment anchor point
        if let Some(prev_hash) = self.previous_hash() {
            let c2pa_boxes = C2PABmffBoxesRollingHash::from_reader(fragment_stream)?;

            // ensure there aren't more than one uuid box
            if c2pa_boxes.rolling_hashes.len() > 1 || c2pa_boxes.bmff_merkle_box_infos.len() > 1 {
                return Err(Error::HashMismatch(
                    "BMFF Fragments shouldn't have more than 1 BmffMerkleMap".to_string(),
                ));
            }

            if let Some(ref_hash) = &c2pa_boxes.rolling_hashes[0].anchor_point {
                if *prev_hash != **ref_hash {
                    return Err(Error::HashMismatch(
                        "Previous Hash does not match Fragment Anchor Point".to_string(),
                    ));
                }
            }
        }

        // rolling hash
        if let Some(roll_hash) = self.rolling_hash() {
            let exclusions = bmff_to_jumbf_exclusions(fragment_stream, &self.exclusions, true)?;

            let frag_hash = hash_stream_by_alg(&curr_alg, fragment_stream, Some(exclusions), true)?;

            let (left, right) = if let Some(prev_hash) = self.previous_hash() {
                (prev_hash, Some(frag_hash.as_slice()))
            } else {
                (&frag_hash, None)
            };
            let ref_hash = concat_and_hash(&curr_alg, left, right);

            if ref_hash != *roll_hash {
                return Err(Error::HashMismatch(
                    "Fragment Hash does not match Rolling Hash".to_string(),
                ));
            }
        } else {
            return Err(Error::HashMismatch(
                "Asset File has no Rolling Hash".to_string(),
            ));
        }

        Ok(())
    }

    pub fn verify_fragment(
        &self,
        init_stream: &mut dyn CAIRead,
        fragment_stream: &mut dyn CAIRead,
        alg: Option<&str>,
        previous_hash: &[u8],
    ) -> Result<()> {
        // verify Init Hash
        self.verify_stream_hash(init_stream, alg)?;

        let curr_alg = match &self.alg {
            Some(a) => a.clone(),
            None => match alg {
                Some(a) => a.to_owned(),
                None => "sha256".to_string(),
            },
        };

        let c2pa_boxes = C2PABmffBoxesRollingHash::from_reader(fragment_stream)?;

        // ensure there aren't more than one uuid box
        if c2pa_boxes.rolling_hashes.len() > 1 || c2pa_boxes.bmff_merkle_box_infos.len() > 1 {
            return Err(Error::HashMismatch(
                "BMFF Fragments shouldn't have more than 1 BmffMerkleMap".to_string(),
            ));
        }

        // make sure all three previous hashes match
        if let Some(anchor_point) = &c2pa_boxes.rolling_hashes[0].anchor_point {
            if previous_hash != **anchor_point {
                return Err(Error::HashMismatch(
                    "Anchor point does not match given previous hash".to_string(),
                ));
            }

            let Some(prev_hash) = self.previous_hash() else {
                return Err(Error::HashMismatch(
                    "Manifest is missing previous hash".to_string(),
                ));
            };

            if prev_hash != previous_hash {
                return Err(Error::HashMismatch(
                    "Previous Hashes are mismatched".to_string(),
                ));
            }
        }

        // verify rolling hash
        let exclusions = bmff_to_jumbf_exclusions(fragment_stream, &self.exclusions, true)?;

        let frag_hash = hash_stream_by_alg(&curr_alg, fragment_stream, Some(exclusions), true)?;

        let ref_hash = concat_and_hash(&curr_alg, previous_hash, Some(&frag_hash));

        let Some(roll_hash) = self.rolling_hash() else {
            return Err(Error::HashMismatch(
                "Asset File has no Rolling Hash".to_string(),
            ));
        };

        if ref_hash != *roll_hash {
            return Err(Error::HashMismatch(
                "Fragment Hash does not match Rolling Hash".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate a whole Rolling Hash set, beginning at the very first
    /// fragment in the stream and ending with the fragment referenced
    /// in the Init Fragment.
    // TODO not verified to be working, but also not important for the testbed
    #[cfg(feature = "file_io")]
    pub fn verify_stream_fragments(
        &self,
        init_stream: &mut dyn CAIRead,
        fragments: &[PathBuf],
        alg: Option<&str>,
    ) -> Result<()> {
        // verify Init Hash
        self.verify_stream_hash(init_stream, alg)?;

        let curr_alg = match &self.alg {
            Some(a) => a.clone(),
            None => match alg {
                Some(a) => a.to_owned(),
                None => "sha256".to_string(),
            },
        };

        // validate first fragment separately
        let mut fragments = fragments.iter();
        let Some(first) = fragments.next() else {
            return Ok(());
        };
        let mut fp = std::fs::OpenOptions::new().read(true).open(first)?;
        let mut rolling_hash = self.hash_fragment(&mut fp, &curr_alg, None, true)?;

        // roll through all the hashes
        for frag in fragments {
            let mut fp = std::fs::OpenOptions::new().read(true).open(frag)?;
            rolling_hash = self.hash_fragment(&mut fp, &curr_alg, Some(&rolling_hash), false)?;
        }

        // final hash should match rolling hash
        if let Some(ref_hash) = self.rolling_hash() {
            if rolling_hash != *ref_hash {
                return Err(Error::HashMismatch("mismatch rolling hash".to_string()));
            }
        } else {
            return Err(Error::HashMismatch("missing rolling hash".to_string()));
        }

        Ok(())
    }

    /// Validate a RollingHash Fragment with hashes from memory.
    ///
    /// This is only used for the temporary hack to validate
    /// fragments by the client. Until the proper validation
    /// is integrated into WASM or we have our own JS library.
    pub fn verify_fragment_memory(
        &self,
        fragment_stream: &mut dyn CAIRead,
        alg: Option<&str>,
        rolling_hash: &[u8],
        previous_hash: &[u8],
    ) -> Result<()> {
        let curr_alg = match &self.alg {
            Some(a) => a.clone(),
            None => match alg {
                Some(a) => a.to_owned(),
                None => "sha256".to_string(),
            },
        };

        // hash fragment stream
        let exclusions = bmff_to_jumbf_exclusions(fragment_stream, &self.exclusions, true)?;
        let frag_hash = hash_stream_by_alg(&curr_alg, fragment_stream, Some(exclusions), true)?;

        let ref_hash = concat_and_hash(&curr_alg, previous_hash, Some(&frag_hash));

        if ref_hash != rolling_hash {
            return Err(Error::HashMismatch("missing rolling hash".to_string()));
        }

        // TODO
        Ok(())
    }

    fn hash_fragment(
        &self,
        reader: &mut dyn CAIRead,
        alg: &str,
        previous_hash: Option<&[u8]>,
        is_first: bool,
    ) -> Result<Vec<u8>> {
        let c2pa_boxes = C2PABmffBoxesRollingHash::from_reader(reader)?;

        // hash fragment stream
        let exclusions = bmff_to_jumbf_exclusions(reader, &self.exclusions, true)?;
        let frag_hash = hash_stream_by_alg(alg, reader, Some(exclusions), true)?;

        let (left, right) = match (previous_hash, is_first) {
            (Some(ph), false) => {
                if c2pa_boxes.rolling_hashes.len() != 1 {
                    return Err(Error::HashMismatch(
                        "non-first Fragment requires exactly one embedded previous hash"
                            .to_string(),
                    ));
                }

                (ph, Some(frag_hash.as_slice()))
            }
            (Some(_), true) => {
                // TODO maybe use Init Hash as previous for first Fragment?
                return Err(Error::HashMismatch(
                    "first Fragment expects no previous hash".to_string(),
                ));
            }
            (None, false) => {
                return Err(Error::HashMismatch(
                    "non-first Fragment requires previous hash".to_string(),
                ));
            }
            (None, true) => {
                if !c2pa_boxes.rolling_hashes.is_empty() {
                    return Err(Error::HashMismatch(
                        "first Fragment should not have a previous hash embedded".to_string(),
                    ));
                }
                (frag_hash.as_slice(), None)
            }
        };

        Ok(concat_and_hash(alg, left, right))
    }
}

impl AssertionCbor for RollingHash {}

impl AssertionBase for RollingHash {
    const LABEL: &'static str = Self::LABEL;

    fn from_assertion(assertion: &crate::assertion::Assertion) -> Result<Self> {
        Self::from_cbor_assertion(assertion)
    }

    fn to_assertion(&self) -> Result<crate::assertion::Assertion> {
        Self::to_cbor_assertion(self)
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct FragmentRollingHash {
    anchor_point: Option<ByteBuf>,
}
