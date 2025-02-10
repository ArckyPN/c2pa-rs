use std::path::PathBuf;

use anyhow::Result;

#[derive(Debug, Clone)]
pub(crate) struct C2PABuilder {
    pub manifest_json: String,
    pub base_path: PathBuf,
}

impl C2PABuilder {
    pub fn builder(&self) -> Result<c2pa::Builder> {
        let mut builder = c2pa::Builder::from_json(&self.manifest_json)?;
        builder.base_path = Some(self.base_path.clone());
        Ok(builder)
    }

    pub fn signer(&self) -> Result<Box<dyn c2pa::Signer>> {
        let mut config = crate::SignConfig::from_json(&self.manifest_json)?;
        config.set_base_path(self.base_path.clone());
        config.signer()
    }
}
