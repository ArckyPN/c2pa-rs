use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;

#[derive(Debug)]
pub(crate) struct UriInfo {
    pub(crate) rep_id: u8,
    pub(crate) index: FragmentIndex,
}

#[derive(Debug)]
pub(crate) enum FragmentIndex {
    Index(u32),
    Init,
}

#[derive(Debug)]
pub(crate) struct Regexp {
    fragment: Regex,
}

impl Regexp {
    pub fn uri<P>(&self, uri: P) -> Result<UriInfo>
    where
        P: AsRef<Path>,
    {
        let uri = uri.as_ref().to_str().context("invalid URI")?;
        let capture = self.fragment.captures(uri).context("no matches")?;

        let index = match &capture["index"] {
            "init" => FragmentIndex::Init,
            i => FragmentIndex::Index(i.parse()?),
        };

        Ok(UriInfo {
            rep_id: capture["rep"].parse()?,
            index,
        })
    }
}

impl Default for Regexp {
    fn default() -> Self {
        Self {
            fragment: Regex::new(r"fragment_(?P<rep>\d+)_0*(?P<index>\d+|init)\.m4s").unwrap(),
        }
    }
}
