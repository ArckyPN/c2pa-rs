use std::{
    cmp::Ordering,
    fmt::Display,
    iter::FromIterator,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
};

use anyhow::{bail, ensure, Context, Result};
use reqwest::{Body, IntoUrl, Response};
use url::Url;
use utility::{is_fragment, is_init};

pub(crate) mod c2pa_builder;
pub(crate) mod manifest_signer;
pub(crate) mod merkle_tree;
pub(crate) mod regexp;
pub(crate) mod routes;
pub(crate) mod utility;

use c2pa_builder::C2PABuilder;
use regexp::{Regexp, UriInfo};

/// FFmpeg -window_size argument
///
/// TODO ideally set programmatically, i.e. CLI or ENV
pub(super) const SEGMENT_LIST_NUM: usize = 5;

// ! MPD / Server Approach code
/* macro_rules! run_async {
    ($block:tt) => {
        rocket::futures::executor::block_on(async { $block })
    };
    ($call:stmt) => {
        run_async!({ $call })
    };
} */

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) enum ForwardType {
    Manifest,
    Separate,
    Signed,
    RollingHash,
}

impl Display for ForwardType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Manifest => "manifest",
            Self::Separate => "separate",
            Self::Signed => "signed",
            Self::RollingHash => "rolling-hash",
        };
        f.write_str(s)
    }
}

pub(crate) struct LiveSigner {
    /// local directory where to save the stream to
    pub media: PathBuf,

    /// CDN base URL where signed stream is publish to
    pub target: Url,

    /// async `reqwest::Client` used to post to CDN
    pub client: reqwest::Client,

    /// sync `reqwest::blocking::Client` used to post to CDN
    pub sync_client: Arc<reqwest::blocking::Client>,

    /// C2PA signer
    pub c2pa: C2PABuilder,

    /// helper Regex
    pub regex: Arc<Regexp>,

    /// Merkle Tree group size
    pub window_size: usize,
    // ! MPD / Server Approach code
    /* pub cache: Arc<ManifestCache>, */
}

impl LiveSigner {
    /// creates the local path from the ingest URI
    ///
    /// `<media>/<name>/<uri..>`
    pub fn local_path<P>(&self, name: &str, uri: P, ty: Option<ForwardType>) -> PathBuf
    where
        P: AsRef<Path>,
    {
        let name = match ty {
            Some(ty) => format!("{name}_{ty}"),
            None => name.to_owned(),
        };

        self.media.join(name).join(uri)
    }

    /// creates the CDN URL for the given type `ty` of
    /// [ForwardType]
    ///
    /// `<target>/<name>_<type>/<uri..>`
    pub fn cdn_url<P>(&self, name: &str, uri: P, ty: Option<ForwardType>) -> Result<Url>
    where
        P: AsRef<Path>,
    {
        let uri = uri.as_ref().as_os_str().to_str().context("invalid uri")?;

        let uri = match ty {
            Some(t) => format!("{name}_{t}/{uri}"),
            None => format!("{name}/{uri}"),
        };

        Ok(self.target.join(&uri)?)
    }

    /// converts the given init file to its corresponding
    /// output path
    ///
    /// `<media>/signed_<name>/`
    fn output<P>(&self, name: &str, init: P, ty: ForwardType) -> Result<PathBuf>
    where
        P: AsRef<Path>,
    {
        self.path_to_signed_path(name, init, ty)
    }

    /// creates the output directory path of the original content
    ///
    /// `<media>/<name>/`
    fn local(&self, name: &str, rep_id: u8) -> PathBuf {
        self.media.join(name).join(rep_id.to_string())
    }

    /// finds all paths associated with the given uri
    /// used to add this file to the signed stream
    ///
    /// returns (init path, fragment paths)
    fn paths_to_sign<P>(&self, name: &str, uri: P) -> Result<(PathBuf, Vec<PathBuf>)>
    where
        P: AsRef<Path>,
    {
        let mut init = None;
        let mut fragments = Vec::new();

        for path in self.paths(name, uri)? {
            if is_init(&path) {
                match init {
                    Some(_) => bail!("found multiple init files"),
                    None => {
                        init.replace(path);
                    }
                }
            } else {
                fragments.push(path);
            }
        }

        let init = init.context("missing init file")?;

        fragments.sort();

        Ok((init, fragments))
    }

    /// collects all local signed paths + forward CDN URL pairs
    ///
    /// this only includes the last Merkle Tree group, according
    /// to the configured window_size
    ///
    /// returns Vec<(local path, forward URL)>
    fn forward<P>(&self, name: &str, uri: P, ty: ForwardType) -> Result<Vec<(PathBuf, Url)>>
    where
        P: AsRef<Path>,
    {
        let mut pairs = Vec::new();

        for path in self.paths(name, uri)? {
            pairs.push((
                self.path_to_signed_path(name, &path, ty)?,
                self.path_to_cdn_url(path, name, &Some(ty))?,
            ));
        }

        // sort in ascending order, init fragment first
        pairs.sort_by(|a, b| {
            // init always the very first
            if is_init(&a.0) {
                return Ordering::Less;
            }
            if is_init(&b.0) {
                return Ordering::Greater;
            }
            a.0.cmp(&b.0)
        });

        let init = pairs[0].clone();
        ensure!(is_init(&init.0), "first forward pair is not init");

        if self.window_size == 0 {
            return Ok(pairs);
        }

        let mut pairs = match ty {
            // get the fragments for SegmentList
            ForwardType::Manifest => {
                let cutoff = if pairs.len() < SEGMENT_LIST_NUM {
                    1
                } else {
                    pairs.len() - SEGMENT_LIST_NUM
                };
                pairs.split_off(cutoff)
            }
            // get the final group, which is being newly signed
            _ => pairs[1..]
                .chunks(self.window_size)
                .last()
                .context("missing fragments")?
                .to_vec(),
        };

        pairs.push(init);

        // reverse order to have init first and then the newest fragment first
        pairs.reverse();

        Ok(pairs)
    }

    /// converts a local path to the corresponding signed file path
    ///
    /// /path/to/media/<name>/<uri..> -> /path/to/media/<name>_<ty>/<uri..>
    fn path_to_signed_path<P>(&self, name: &str, path: P, ty: ForwardType) -> Result<PathBuf>
    where
        P: AsRef<Path>,
    {
        let parts = path
            .as_ref()
            .to_str()
            .context("invalid path")?
            .split("/")
            .map(|p| {
                if p == name {
                    format!("{name}_{ty}")
                } else {
                    p.to_string()
                }
            });
        Ok(PathBuf::from_iter(parts))
    }

    /// converts a local path to the corresponding CDN URL
    ///
    /// /path/to/media/<uri..> -> http://<target..>/<uri..>
    fn path_to_cdn_url<P>(&self, path: P, name: &str, ty: &Option<ForwardType>) -> Result<Url>
    where
        P: AsRef<Path>,
    {
        let uri = path
            .as_ref()
            .strip_prefix(&self.media)?
            .to_str()
            .context("failed strip prefix")?;
        let uri = match ty {
            Some(t) => &uri.replace(name, &format!("{name}_{t}")),
            None => uri,
        };
        Ok(self.target.join(uri)?)
    }

    // ! MPD / Server Approach code
    /* fn separate_to_c2pa_url<U>(&self, forward: U) -> Result<Url>
    where
        U: IntoUrl,
    {
        let url = forward.as_str();
        let url = url.replace("ingest", "c2pa").replace("_separate", "");
        Ok(Url::parse(&url)?)
    } */

    /// reads all paths associated with the same RepID
    fn paths<P>(&self, name: &str, uri: P) -> Result<Vec<PathBuf>>
    where
        P: AsRef<Path>,
    {
        let mut paths = Vec::new();
        let UriInfo { rep_id, index: _ } = self.regex.uri(uri)?;

        for entry in self.local(name, rep_id).read_dir()? {
            let entry = entry?;
            let path = entry.path();

            if !is_fragment(&path) {
                continue;
            }
            let UriInfo {
                rep_id: comp,
                index: _,
            } = self.regex.uri(&path)?;
            if rep_id != comp {
                continue;
            }

            paths.push(path);
        }

        Ok(paths)
    }

    pub async fn post<U, T>(&self, url: U, body: Option<T>) -> Result<Response>
    where
        U: IntoUrl,
        T: Into<Body>,
    {
        let res = match body {
            Some(body) => self.client.post(url).body(body).send().await?,
            None => self.client.post(url).send().await?,
        };
        Ok(res)
    }

    pub async fn delete<U>(&self, url: U) -> Result<Response>
    where
        U: IntoUrl,
    {
        let res = self.client.delete(url).send().await?;
        Ok(res)
    }

    // ! MPD / Server Approach code
    /* fn forward_to_uuid_forward(&self, forward: &[(PathBuf, Url)]) -> Result<Vec<Url>> {
        let mut vec = Vec::new();
        for (_, url) in forward {
            vec.push(self.separate_to_c2pa_url(url.clone())?);
        }
        Ok(vec)
    } */

    fn rolling_hash_input_paths<P>(&self, name: &str, uri: P) -> Result<(PathBuf, PathBuf)>
    where
        P: AsRef<Path>,
    {
        let init = self
            .paths(name, &uri)?
            .iter()
            .find(|p| is_init(p))
            .context("missing init file")?
            .to_owned();

        let fragment = self.local_path(name, uri, None);

        Ok((init, fragment))
    }

    fn rolling_hash_forward_urls<P1, P2>(
        &self,
        name: &str,
        init: P1,
        fragment: P2,
    ) -> Result<Vec<(PathBuf, Url)>>
    where
        P1: AsRef<Path>,
        P2: AsRef<Path>,
    {
        let mut vec = Vec::new();

        let fragment_url =
            self.path_to_cdn_url(&fragment, name, &Some(ForwardType::RollingHash))?;
        let fragment_path = self.path_to_signed_path(name, &fragment, ForwardType::RollingHash)?;

        vec.push((fragment_path, fragment_url));

        let init_url = self.path_to_cdn_url(&init, name, &Some(ForwardType::RollingHash))?;
        let init_path = self.path_to_signed_path(name, &init, ForwardType::RollingHash)?;

        vec.push((init_path, init_url));

        Ok(vec)
    }

    pub async fn sign<P>(&self, name: &str, uri: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        // ! MPD / Server Approach code
        /* let separate_forward = self.forward(name, &uri, Some(ForwardType::Separate))?;
        let manifest_forward = self
            .forward(name, &uri, Some(ForwardType::Manifest))?
            .iter()
            .map(|f| f.0.clone())
            .collect::<Vec<PathBuf>>();
        let uuid_forward = self.forward_to_uuid_forward(&separate_forward)?;
        let manifest_signer = self.cache.clone(); */

        // Rolling Hash signing

        // let UriInfo { rep_id, index: _ } = self.regex.uri(&uri)?;

        let builder = self.c2pa.clone();
        let (init, fragment) = self.rolling_hash_input_paths(name, &uri)?;
        // let output_dir = self.local_path(name, rep_id.to_string(), Some(ForwardType::RollingHash));
        let output = self.output(name, &init, ForwardType::RollingHash)?;
        let signed_forward = self.rolling_hash_forward_urls(name, &init, &fragment)?;
        let client = self.sync_client.clone();
        thread::Builder::new()
            .name(format!("Rolling Hash {name} - {:?}", uri.as_ref()))
            .spawn(move || -> Result<()> {
                let signer = builder.signer()?;
                let mut c2pa = builder.builder()?;

                // sign
                if let Err(err) =
                    c2pa.sign_live_bmff(signer.as_ref(), init, &vec![fragment], output, None)
                {
                    log::error!("Sign: {err}");
                    bail!("Sign: {err}")
                }

                // forward signed fragments to signed
                for (path, url) in signed_forward {
                    let buf = std::fs::read(path)?;
                    client.post(url).body(buf).send()?;
                }

                Ok(())
            })?;

        // Optimized Merkle Tree signing

        let (init, fragments) = self.paths_to_sign(name, &uri)?;
        let output = self.output(name, &init, ForwardType::Signed)?;
        let signed_forward = self.forward(name, &uri, ForwardType::Signed)?;
        let client = self.sync_client.clone();
        let window_size = self.window_size;
        let builder = self.c2pa.clone();
        thread::Builder::new()
            .name(format!("Merkle: {name} - {:?}", uri.as_ref()))
            .spawn(move || -> Result<()> {
                let signer = builder.signer()?;
                let mut c2pa = builder.builder()?;

                if window_size == 0 {
                    clear_dir(&output)?;
                }

                // sign
                if let Err(err) = c2pa.sign_live_bmff(
                    signer.as_ref(),
                    init,
                    &fragments,
                    output,
                    Some(window_size),
                ) {
                    log::error!("Sign: {err}");
                    bail!("Sign: {err}")
                }

                // forward signed fragments to signed
                for (path, url) in signed_forward {
                    // println!("Merkle: {path:?} {}", path.exists());
                    let buf = std::fs::read(path)?;
                    client.post(url).body(buf).send()?;
                }

                // ! MPD / Server Approach code
                /* // only cache the uuid boxes of the fragments that
                // will be listed in the Manifests
                if let Some((media, url)) = run_async!({
                    let init = &manifest_forward[0];

                    // reverse order to have the segment in chronological order
                    let mut manifest_forward = manifest_forward[1..].to_vec();
                    manifest_forward.reverse();

                    manifest_signer
                        .insert_segment_list(init, &manifest_forward)
                        .await
                })? {
                    // forward MediaPlaylist
                    client.post(url).body(media).send()?;
                }

                // forward MPD
                if let Some((mpd, url)) = run_async!(manifest_signer.mpd_ready().await) {
                    client.post(url).body(mpd).send()?;
                }

                // save separated UUID Boxes on server (here: also CDN for simplicity)
                for ((path, url), c2pa_url) in separate_forward.into_iter().zip(uuid_forward) {
                    let uuid = extract_c2pa_box(&path)?;
                    // TODO write c2pa_url into manifests (like other approach instead of into uuid box) - this will save space by not having the life a third time
                    let fragment = replace_uuid_content(path, c2pa_url.as_str().as_bytes())?;

                    client.post(c2pa_url).body(uuid).send()?;
                    client.post(url).body(fragment).send()?;
                } */

                Ok(())
            })?;

        Ok(())
    }
}

fn clear_dir<P>(init: P) -> Result<()>
where
    P: AsRef<Path>,
{
    let dir = init.as_ref().parent().context("missing dir")?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
