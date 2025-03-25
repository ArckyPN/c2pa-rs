use std::{path::PathBuf, str::FromStr};

use anyhow::Result;
use c2pa::{Signer, SigningAlg, create_signer};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Signing algorithm to use - must match the associated certs
    ///
    /// Must be one of [ ps256 | ps384 | ps512 | es256 | es384 | es512 | ed25519 ]
    /// Defaults to es256
    pub alg: String,
    /// A path to a file containing the private key required for signing
    pub private_key: PathBuf,
    /// A path to a file containing the signing cert required for signing
    pub sign_cert: PathBuf,
    /// A Url to a Time Authority to use when signing the manifest
    pub ta_url: String,
}

impl Config {
    pub fn from_json(json: &str) -> Result<Box<dyn Signer>> {
        let this: Self = serde_json::from_str(json)?;

        Ok(create_signer::from_files(
            &this.sign_cert,
            &this.private_key,
            SigningAlg::from_str(&this.alg)?,
            Some(this.ta_url),
        )?)
    }
}
