#![allow(dead_code)]
use std::{fmt::Display, path::Path, str::FromStr};

use anyhow::{bail, Context, Error, Result};
use regex::Regex;

#[derive(Debug)]
pub(crate) struct UriInfo {
    pub(crate) rep_id: u8,
    pub(crate) index: FragmentIndex,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FragmentIndex {
    Index(u32),
    Init,
    Manifest(ManifestTypes),
}

impl Display for FragmentIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Index(idx) => f.write_str(&idx.to_string()),
            Self::Init => f.write_str("init"),
            Self::Manifest(m) => f.write_str(&m.to_string()),
        }
    }
}

impl FromStr for FragmentIndex {
    type Err = Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "init" => Ok(Self::Init),
            "mpd" => Ok(Self::Manifest(ManifestTypes::Mpd)),
            "media" => Ok(Self::Manifest(ManifestTypes::Media)),
            "master" => Ok(Self::Manifest(ManifestTypes::Master)),
            x => Ok(Self::Index(x.parse()?)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ManifestTypes {
    Mpd,
    Master,
    Media,
}

impl Display for ManifestTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Master => f.write_str("master"),
            Self::Mpd => f.write_str("mpd"),
            Self::Media => f.write_str("media"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Regexp {
    fragment: Regex,
    playlist: Regex,
}

impl Regexp {
    pub fn uri<P>(&self, uri: P) -> Result<UriInfo>
    where
        P: AsRef<Path>,
    {
        let uri = uri.as_ref().to_str().context("invalid URI")?;
        let capture = self.fragment.captures(uri).context("no matches uri")?;

        let index = match &capture["index"] {
            "init" => FragmentIndex::Init,
            i => FragmentIndex::Index(i.parse()?),
        };

        Ok(UriInfo {
            rep_id: capture["rep"].parse()?,
            index,
        })
    }

    pub fn manifest<P>(&self, url: P) -> Result<UriInfo>
    where
        P: AsRef<Path>,
    {
        let url = url.as_ref().to_string_lossy().to_string();
        if url.contains(".mpd") {
            Ok(UriInfo {
                rep_id: 0,
                index: FragmentIndex::Manifest(ManifestTypes::Mpd),
            })
        } else if url.contains("master.m3u8") {
            Ok(UriInfo {
                rep_id: 0,
                index: FragmentIndex::Manifest(ManifestTypes::Master),
            })
        } else if url.contains("media_") {
            let capture = self
                .playlist
                .captures(&url)
                .context("no matches manifest")?;

            Ok(UriInfo {
                rep_id: capture["rep"].parse()?,
                index: FragmentIndex::Manifest(ManifestTypes::Media),
            })
        } else {
            bail!("invalid manifest url")
        }
    }
}

impl Default for Regexp {
    fn default() -> Self {
        Self {
            fragment: Regex::new(r"(?P<rep>\d+)/segment_0*(?P<index>\d+|init)\.m4s").unwrap(),
            playlist: Regex::new(r"media_(?P<rep>\d+)\.m3u8").unwrap(),
        }
    }
}
