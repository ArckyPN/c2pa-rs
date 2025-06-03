use std::path::PathBuf;

use c2pa_crypto::base64;
use dash_mpd::{Event, EventStream};
use rocket::{http::Status, Data, State};

use crate::{
    live::{
        regexp::{FragmentIndex, ManifestTypes, UriInfo},
        ROLLING_HASH_SCHEME_URI,
    },
    log_err,
};

use super::{
    utility::{is_init, process_request_body},
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

    if let Ok(UriInfo { rep_id: _, index }) = state.regex.manifest(&uri) {
        // this is a manifest request

        // insert C2PA data into Manifests
        let res = match index {
            FragmentIndex::Manifest(ManifestTypes::Mpd) => {
                // TODO put this in the LiveSigner
                let xml = log_err!(String::from_utf8(buf), "MPD payload not UTF-8")?;
                let mut mpd = log_err!(dash_mpd::parse(&xml), "parse MPD")?;

                for period in mpd.periods.as_mut_slice() {
                    let mut event = Vec::new();
                    for adaptation in period.adaptations.as_mut_slice() {
                        for representation in adaptation.representations.as_mut_slice() {
                            let Some(rep_id) = &representation.id else {
                                continue;
                            };

                            let json =
                                log_err!(state.manifold.get_json(rep_id).await, "fetch c2pa data")?;

                            event.push(Event {
                                id: Some(rep_id.to_owned()),
                                presentationTime: None,
                                presentationTimeOffset: None,
                                duration: None,
                                timescale: None,
                                contentEncoding: Some("base64".to_string()),
                                messageData: Some(base64::encode(&json)),
                                SelectionInfo: None,
                                signal: Vec::new(),
                                splice_info_section: Vec::new(),
                                value: None,
                                content: None,
                            });
                        }
                    }
                    period.event_streams.push(EventStream {
                        // reference to an external EventStream element
                        href: None,
                        // only used when href is Some(...)
                        actuate: None,
                        // this is not listed in the spec?
                        messageData: None,
                        // message scheme
                        schemeIdUri: ROLLING_HASH_SCHEME_URI.to_string(),
                        // value specified by schemeIdUri
                        value: None,
                        // units per seconds used by Events
                        timescale: None,
                        // time offset for this period
                        presentationTimeOffset: None,
                        // the actual Events
                        event,
                    });
                }

                let s = mpd.to_string();
                s.as_bytes().to_vec()
            }
            FragmentIndex::Manifest(ManifestTypes::Master) => buf,
            FragmentIndex::Manifest(ManifestTypes::Media) => {
                // TODO HLS Event stream signaling (ala Ad-Insertion)
                buf
            }
            _ => unreachable!("{} is not possible", index),
        };

        // post Manifests to CDN
        let url = log_err!(
            state.cdn_url(name, &uri, Some(crate::live::ForwardType::RollingHash)),
            "cdn url RollingHash"
        )?;
        log_err!(
            state.post(url, Some(res)).await,
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
