use std::{
    cmp::Ordering,
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
pub(crate) mod regexp;
pub(crate) mod routes;
pub(crate) mod utility;

use c2pa_builder::C2PABuilder;
use regexp::{FragmentIndex, Regexp, UriInfo};

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
}

impl LiveSigner {
    /// creates the local path from the ingest URI
    ///
    /// `<media>/<name>/<uri..>`
    pub fn local_path<P>(&self, name: &str, uri: P) -> PathBuf
    where
        P: AsRef<Path>,
    {
        self.media.join(name).join(uri)
    }

    /// creates the CDN URL from the ingest URI
    ///
    /// `<target>/<name>/<uri..>`
    pub fn cdn_url<P>(&self, name: &str, uri: P) -> Result<Url>
    where
        P: AsRef<Path>,
    {
        let uri = uri.as_ref().as_os_str().to_str().context("invalid uri")?;

        Ok(self.target.join(&format!("{name}/{uri}"))?)
    }

    fn cdn_init_cache(&self, name: &str) -> Url {
        let mut url = self.target.clone();
        url.set_path(&format!("init/{name}"));
        url
    }

    /// creates the output directory path of the signed content
    ///
    /// `<media>/signed_<name>/`
    fn output<P>(&self, name: &str, init: P) -> Result<PathBuf>
    where
        P: AsRef<Path>,
    {
        self.path_to_signed_path(name, init)
    }

    /// creates the output directory path of the original content
    ///
    /// `<media>/<name>/`
    fn local(&self, name: &str) -> PathBuf {
        self.media.join(name)
    }

    /// finds all associated init and fragment paths
    ///
    /// returns (init path, fragment paths)
    fn signed_paths<P>(&self, name: &str, uri: P) -> Result<(PathBuf, Vec<PathBuf>)>
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
    /// returns Vec<(local path, forward URL)>
    fn forward<P>(&self, name: &str, uri: P) -> Result<Vec<(PathBuf, Url)>>
    where
        P: AsRef<Path>,
    {
        let mut pairs = Vec::new();

        for path in self.paths(name, uri)? {
            pairs.push((
                self.path_to_signed_path(name, &path)?,
                self.path_to_cdn_url(path)?,
            ));
        }

        pairs.sort_by(|a, b| {
            // init always the very first
            if is_init(&a.0) {
                return Ordering::Less;
            }
            if is_init(&b.0) {
                return Ordering::Greater;
            }
            b.0.cmp(&a.0)
        });

        Ok(pairs)
    }

    /// converts a local path to the corresponding signed file path
    ///
    /// /path/to/media/<name>/<uri..> -> /path/to/media/signed_<name>/<uri..>
    fn path_to_signed_path<P>(&self, name: &str, path: P) -> Result<PathBuf>
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
                    format!("signed_{name}")
                } else {
                    p.to_string()
                }
            });
        Ok(PathBuf::from_iter(parts))
    }

    /// converts a local path to the corresponding CDN URL
    ///
    /// /path/to/media/<uri..> -> http://<target..>/<uri..>
    fn path_to_cdn_url<P>(&self, path: P) -> Result<Url>
    where
        P: AsRef<Path>,
    {
        let uri = path
            .as_ref()
            .strip_prefix(&self.media)?
            .to_str()
            .context("failed strip prefix")?;
        Ok(self.target.join(uri)?)
    }

    /// reads all paths associated with the same RepID
    fn paths<P>(&self, name: &str, uri: P) -> Result<Vec<PathBuf>>
    where
        P: AsRef<Path>,
    {
        let mut paths = Vec::new();
        let UriInfo { rep_id, index: _ } = self.regex.uri(uri)?;

        for entry in self.local(name).read_dir()? {
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

    pub fn sign<P>(&self, name: &str, uri: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let thread_name = format!("{name} - {:?}", uri.as_ref());
        let (init, fragments) = self.signed_paths(name, &uri)?;
        let output = self.output(name, &init)?;
        let forward = self.forward(name, uri)?;
        let client = self.sync_client.clone();
        let re = self.regex.clone();
        let cdn = self.cdn_init_cache(name);

        ensure!(
            forward.len() >= 2,
            "forward pairs must have at least two pairs, one init and one (or more) fragments"
        );

        let builder = self.c2pa.clone();

        thread::Builder::new()
            .name(thread_name)
            .spawn(move || -> Result<()> {
                let signer = builder.signer()?;
                let mut c2pa = builder.builder()?;

                clear_output(&output)?;
                c2pa.sign_fragmented_files(signer.as_ref(), init, &fragments, output)?;

                post_reference_init(&forward, cdn, client.clone(), re.clone())?;

                for (path, url) in forward {
                    let buf = std::fs::read(path)?;

                    // TODO chunked transfer
                    client.post(url).body(buf).send()?;
                }

                Ok(())
            })?;

        Ok(())
    }
}

fn post_reference_init(
    forward: &[(PathBuf, Url)],
    origin: Url,
    client: Arc<reqwest::blocking::Client>,
    re: Arc<Regexp>,
) -> Result<()> {
    let mut cdn = origin;
    let init = &forward[0].0;
    let newest_fragment = &forward[1].0;
    let UriInfo { rep_id, index } = re.uri(newest_fragment)?;

    let init = std::fs::read(init)?;
    let index = match index {
        FragmentIndex::Index(i) => i,
        FragmentIndex::Init => bail!("second fragment should not be the init"),
    };
    cdn.set_query(Some(&format!("rep={rep_id}&index={index}")));
    client.post(cdn).body(init).send()?;
    Ok(())
}

fn clear_output<P>(init: P) -> Result<()>
where
    P: AsRef<Path>,
{
    let dir = init.as_ref().parent().context("output has no parent")?;
    if std::fs::exists(dir)? {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}
