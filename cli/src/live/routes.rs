use std::path::PathBuf;

use rocket::{http::Status, Data, State};

use super::{
    utility::{is_fragment, is_init, process_request_body},
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
    let local = state.local_path(name, &uri);

    let buf = process_request_body(body, local).await.map_err(|err| {
        log::error!("processing request body: {err}");
        Status::InternalServerError
    })?;

    if !is_fragment(&uri) {
        let target = state.cdn_url(name, &uri).map_err(|err| {
            log::error!("building CDN URL: {err}");
            Status::InternalServerError
        })?;

        // forward non-fragment (mpd/playlists) without alterations
        state.post(target, Some(buf)).await.map_err(|err| {
            log::error!("forwarding {name}/{} to CDN: {err}", uri.display());
            Status::InternalServerError
        })?;
        return Ok(());
    }

    if is_init(&uri) {
        // skip init, need at least one fragment to continue to signing
        return Ok(());
    }

    state.sign(name, uri).map_err(|err| {
        log::error!("signing fragment: {err}");
        Status::InternalServerError
    })
}

#[rocket::delete("/<name>/<uri..>")]
pub(crate) async fn delete_ingest(
    name: &str,
    uri: PathBuf,
    state: &State<LiveSigner>,
) -> Result<()> {
    let target = state.cdn_url(name, &uri).map_err(|err| {
        log::error!("building CDN URL: {err}");
        Status::InternalServerError
    })?;

    state.delete(target).await.map_err(|err| {
        log::error!("deleting {name}/{} on CDN: {err}", uri.display());
        Status::InternalServerError
    })?;

    Ok(())
}
