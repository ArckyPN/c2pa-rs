use std::path::PathBuf;

use rocket::{http::Status, serde::json::Json, Data, State};

use super::{
    merkle_tree::MerkleTree,
    utility::{is_fragment, is_init, process_request_body},
    LiveSigner,
};

pub(super) type Result<T> = core::result::Result<T, Status>;

macro_rules! log_err {
    ($fn:expr, $name:expr) => {
        $fn.map_err(|err| {
            log::error!("{}: {err}", $name);
            Status::InternalServerError
        })
    };
    ($fn:expr, $name:expr, $err:expr) => {
        $fn.map_err(|err| {
            log::error!("{}: {err}", $name);
            $err
        })
    };
}

#[rocket::post("/<name>/<uri..>", data = "<body>")]
pub(crate) async fn post_ingest(
    name: &str,
    uri: PathBuf,
    body: Data<'_>,
    state: &State<LiveSigner>,
) -> Result<()> {
    let local = state.local_path(name, &uri);

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
