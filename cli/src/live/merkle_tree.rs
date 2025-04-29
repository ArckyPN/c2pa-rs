use std::{fs::File, path::Path};

use anyhow::{bail, ensure, Context, Result};
use c2pa::{
    assertions::{self, BmffHash},
    asset_handlers::bmff_io::{bmff_to_jumbf_exclusions, read_bmff_c2pa_boxes},
    hash_stream_by_alg,
    utils::hash_utils::concat_and_hash,
    Reader,
};
use c2pa_crypto::base64;
use serde::Serialize;

use super::regexp::{FragmentIndex, UriInfo};

#[derive(Debug, Serialize)]
pub struct MerkleTree {
    init: MerkleTreeInit,
    tree: Vec<Vec<Option<MerkleTreeNode>>>,
}

impl MerkleTree {
    pub fn _new<P>(name: &str, info: UriInfo, media: P, window_size: usize) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let idx = match &info.index {
            FragmentIndex::Index(i) => i.to_owned() as usize,
            _ => bail!("cannot build MerkleTree from init Segment"),
        };
        let rep = info.rep_id;

        // TODO just set any default for init and replace in on the client with the verification result
        let init_path = media
            .as_ref()
            .join(format!("signed_{name}/{rep}/segment_init.m4s"));

        let rem = idx % window_size;
        let from = idx - rem + 1;
        let to = idx + (window_size - rem);

        let mut fragment_paths = Vec::new();
        for i in from..=to {
            fragment_paths.push(
                media
                    .as_ref()
                    .join(format!("signed_{name}/{rep}/segment_{i:09}.m4s")),
            );
        }

        if let Err(err) = Reader::from_fragmented_files(&init_path, &fragment_paths[..rem].to_vec())
        {
            log::error!("Frag: {err}");
        }
        if let Err(err) = Reader::from_file(
            media
                .as_ref()
                .join(format!("signed_{name}/{rep}/segment_init.m4s")),
        ) {
            log::error!("File: {err}");
        }

        // read Init Manifest
        let init = Reader::from_file(
            media
                .as_ref()
                .join(format!("signed_{name}/{rep}/segment_init.m4s")),
        )?;
        let init = init.active_manifest().context("missing active manifest")?;

        let bmff_hash: BmffHash = init.find_assertion(assertions::labels::BMFF_HASH_2)?;
        let merkle = bmff_hash.merkle().context("missing MerkleMaps")?;

        // FIXME don't like this
        let merkle_idx = (idx - 1) / window_size;
        ensure!(
            merkle.len() - 1 == merkle_idx,
            "Merkle does not have enough Trees, expected {} got {}",
            merkle.len() - 1,
            merkle_idx
        );
        let merkle = &merkle[merkle_idx];

        let init = MerkleTreeInit {
            count: merkle.count,
            init_hash: base64::encode(&merkle.init_hash.clone().context("missing init hash")?),
            unique_id: merkle.unique_id,
            local_id: merkle.local_id,
            merkle: merkle
                .hashes
                .iter()
                .map(|hash| base64::encode(hash))
                .collect(),
        };

        let mut leaves = Vec::new();
        for i in from..=to {
            let Ok(mut file) = File::open(
                media
                    .as_ref()
                    .join(format!("signed_{name}/{rep}/segment_{i:09}.m4s")),
            ) else {
                leaves.push(None);
                continue;
            };
            let leave = read_bmff_c2pa_boxes(&mut file)?;

            ensure!(
                leave.bmff_merkle.len() == 1,
                "Fragment must have exactly 1 MerkleTree"
            );

            let leave = &leave.bmff_merkle[0];

            let proofs = match &leave.hashes {
                Some(hashes) => hashes.iter().map(|hash| base64::encode(hash)).collect(),
                None => vec!["-NONE-".to_string()],
            };

            let exclusions = bmff_to_jumbf_exclusions(&mut file, bmff_hash.exclusions(), true)?;
            let hash = hash_stream_by_alg(
                bmff_hash.alg().context("missing algorithm")?,
                &mut file,
                Some(exclusions),
                true,
            )?;

            leaves.push(Some(MerkleTreeNode {
                hash: base64::encode(&hash),
                proofs: Some(proofs),
                name: format!("Fragment {i}"),
                is_current: Some(i == idx),
            }));
        }

        let mut num = leaves.len();
        let mut tree = vec![leaves];
        let mut current = 0;
        loop {
            let mut layer = Vec::new();
            'i: for (i, left) in tree[current].iter().step_by(2).enumerate() {
                let Some(left) = left else {
                    break 'i;
                };
                match &tree[current][i + 1] {
                    Some(right) => {
                        layer.push(Some(MerkleTreeNode {
                            hash: base64::encode(&concat_and_hash(
                                bmff_hash.alg().context("missing alg")?,
                                &base64::decode(&left.hash)?,
                                Some(&base64::decode(&right.hash)?),
                            )),
                            proofs: None,
                            name: format!("Hash {num}"),
                            is_current: None,
                        }));
                        num += 1;
                    }
                    None => layer.push(Some(MerkleTreeNode {
                        hash: left.hash.clone(),
                        proofs: None,
                        name: left.name.clone(),
                        is_current: None,
                    })),
                }
            }
            tree.push(layer);
            current += 1;
            if tree[current].len() == 1 {
                break;
            }
        }

        Ok(Self { init, tree })
    }
}

#[derive(Debug, Serialize)]
pub struct MerkleTreeInit {
    count: u32,
    merkle: Vec<String>,
    unique_id: u32,
    local_id: u32,
    init_hash: String,
}

#[derive(Debug, Serialize)]
pub struct MerkleTreeNode {
    hash: String,
    proofs: Option<Vec<String>>,
    name: String,
    is_current: Option<bool>,
}
