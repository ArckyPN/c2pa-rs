use std::{io::Write, path::PathBuf};

use c2pa_crypto::base64;
use rocket::{
    data::ByteUnit, http::Status, serde::json::Json, tokio::io::AsyncReadExt, Data, State,
};

use crate::log_err;

use super::{
    merkle_tree::MerkleTree,
    utility::{find_init, is_fragment, is_init, process_request_body},
    LiveSigner,
};

pub(super) type Result<T> = core::result::Result<T, Status>;

#[rocket::post("/<name>/<uri..>", data = "<body>")]
pub(crate) async fn post_ingest(
    name: &str,
    uri: PathBuf,
    body: Data<'_>,
    state: &State<LiveSigner>,
) -> Result<()> {
    let local = state.local_path(name, &uri, None);

    // read body and save to local disk

    let buf = log_err!(
        process_request_body(body, local).await,
        "process request body"
    )?;

    // forward everything unchanged
    let url = log_err!(state.cdn_url(name, &uri, None), "cdn url <None>")?;
    log_err!(state.post(url, Some(buf.clone())).await, "post OG content")?;

    if !is_fragment(&uri) {
        // ! MPD / Server Approach code
        /* if state.cache.has_manifests().await {
            return Ok(());
        }
        // cache baseline Manifests to expand by hand
        let manifest_url = log_err!(
            state.cdn_url(name, &uri, Some(ForwardType::Manifest)),
            "cdn url <Manifest>"
        )?;

        let UriInfo { rep_id, index } = log_err!(
            state.regex.manifest(manifest_url.as_str()),
            "regex manifest"
        )?;

        match index {
            FragmentIndex::Manifest(ManifestTypes::Mpd) => {
                let mpd = log_err!(dash_mpd::parse(&String::from_utf8_lossy(&buf)), "parse MPD")?;

                if state.cache.num_reps().await == mpd_num_reps(&mpd) {
                    state.cache.set_mpd(mpd, manifest_url).await;
                }
            }
            FragmentIndex::Manifest(ManifestTypes::Media) => {
                let mut media = log_err!(
                    m3u8_rs::parse_media_playlist_res(&buf),
                    "parse MediaPlaylist"
                )?;
                // remove unknown because it incorrectly parses program time
                media.segments[0].unknown_tags.clear();

                state.cache.insert_media(rep_id, media, manifest_url);
            }
            _ => {}
        } */
        // TODO placeholder until stuff is properly in manifests
        let url = log_err!(
            state.cdn_url(name, &uri, Some(crate::live::ForwardType::RollingHash)),
            "cdn url RollingHash"
        )?;
        log_err!(
            state.post(url, Some(buf.clone())).await,
            "post RollingHash manifests"
        )?;
        return Ok(());
    }

    if is_init(&uri) {
        // ! MPD / Server Approach code
        /* state.cache.add_rep().await; */
        // skip init, need at least one fragment for signing
        return Ok(());
    }

    log_err!(state.sign(name, uri).await, "signing fragment")
}

#[rocket::delete("/<name>/<uri..>")]
pub(crate) async fn delete_ingest(
    name: &str,
    uri: PathBuf,
    state: &State<LiveSigner>,
) -> Result<()> {
    let target = log_err!(state.cdn_url(name, &uri, None), "cdn url <None>")?;

    log_err!(state.delete(target).await, "forward delete")?;

    Ok(())
}

fn unscramble_base64(s: &str) -> String {
    s.replace(".", "+").replace("_", "/").replace("-", "=")
}

#[rocket::post("/?<rep>&<name>&<rolling_hash>&<previous_hash>", data = "<body>")]
pub(crate) async fn verify_rolling_hash(
    name: &str,
    rep: u8,
    rolling_hash: &str,
    previous_hash: &str,
    body: Data<'_>,
    state: &State<LiveSigner>,
) -> Result<String> {
    let mut fragment = Vec::new();
    let mut body = body.open(ByteUnit::max_value());
    log_err!(body.read_to_end(&mut fragment).await, "read verify body")?;

    let dir = state.local(&format!("{name}_rolling-hash"), rep);
    let init_path = log_err!(find_init(dir), "find init")?;
    let mut init_fp = log_err!(
        std::fs::OpenOptions::new().read(true).open(init_path),
        "open init file"
    )?;
    let mut fragment_fp = log_err!(tempfile::NamedTempFile::new(), "create fragment tempfile")?;
    log_err!(
        fragment_fp.write_all(&fragment),
        "write fragment to tempfile"
    )?;

    let rolling_hash = log_err!(
        base64::decode(&unscramble_base64(rolling_hash)),
        "decode rolling hash"
    )?;
    let previous_hash = log_err!(
        base64::decode(&unscramble_base64(previous_hash)),
        "decode previous hash"
    )?;

    let verifier = log_err!(
        c2pa::Reader::from_rolling_hash_memory(
            "m4s",
            &mut init_fp,
            &mut fragment_fp,
            &rolling_hash,
            &previous_hash
        ),
        "verify rolling hash hack"
    )?;

    Ok(verifier.json())
}

#[rocket::get("/merkle-tree/<name>/<uri..>")]
pub(crate) async fn _get_merkle_tree(
    name: &str,
    uri: PathBuf,
    state: &State<LiveSigner>,
) -> Result<Json<MerkleTree>> {
    let info = log_err!(state.regex.uri(&uri), "regex uri")?;

    let tree = log_err!(
        MerkleTree::_new(name, info, &state.media, state.window_size),
        "new MerkleTree"
    )?;

    Ok(Json(tree))
}
