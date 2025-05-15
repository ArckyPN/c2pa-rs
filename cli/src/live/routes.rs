use std::path::PathBuf;

use rocket::{http::Status, Data, State};

use crate::log_err;

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
