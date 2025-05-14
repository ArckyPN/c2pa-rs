#![allow(dead_code)]
use std::{
    fmt::Debug,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use c2pa_crypto::base64;
use dash_mpd::{Initialization, SegmentURL, MPD};
use dashmap::DashMap;
use itertools::{EitherOrBoth, Itertools};
use m3u8_rs::{MediaPlaylist, MediaSegment};
use rocket::tokio::sync::RwLock;
use url::Url;

use super::{
    regexp::{Regexp, UriInfo},
    utility::extract_c2pa_box,
};

type Shared<T> = Arc<RwLock<T>>;

#[derive(Debug, Default)]
pub struct ManifestCache {
    mpd: Shared<Option<(MPD, Url)>>,
    media: DashMap<u8, (MediaPlaylist, Url)>,

    num_reps: Shared<usize>,

    re: Arc<Regexp>,
}

impl ManifestCache {
    pub fn new(re: Arc<Regexp>) -> Self {
        Self {
            re,
            ..Default::default()
        }
    }

    pub async fn has_manifests(&self) -> bool {
        self.mpd.read().await.is_some() && self.media.len() == self.num_reps().await
    }

    pub async fn set_mpd(&self, mpd: MPD, url: Url) {
        let mut lock = self.mpd.write().await;
        lock.replace((mpd, url));
    }

    pub fn insert_media(&self, rep_id: u8, media: MediaPlaylist, url: Url) {
        self.media.insert(rep_id, (media, url));
    }

    /// Insert Init and Fragments into SegmentList.
    ///
    /// Returns serialized MediaPlaylist + Url.
    pub async fn insert_segment_list<P>(
        &self,
        init: P,
        paths: &[PathBuf],
    ) -> Result<Option<(Vec<u8>, Url)>>
    where
        P: AsRef<Path>,
    {
        self.insert_mpd_segment_list(&init, paths).await?;

        self.insert_media_playlist_segment_list(init, paths)
    }

    pub async fn insert_mpd_segment_list<P>(&self, init: P, paths: &[PathBuf]) -> Result<()>
    where
        P: AsRef<Path>,
    {
        // let UriInfo { rep_id, index: _ } = self.re.uri(&init)?;

        // if let Some((mpd, _)) = self.mpd.write().await.as_mut() {
        //     mpd.publishTime = Some(Self::now()?);
        //     mpd.suggestedPresentationDelay = Some(Duration::from_secs(5));

        //     for period in mpd.periods.iter_mut() {
        //         for adaptation in period.adaptations.iter_mut() {
        //             for representation in adaptation.representations.iter_mut() {
        //                 // TODO alternatively better to use InbandEventStream to be standard conform
        //                 let Some(id) = &representation.id else {
        //                     unreachable!("RepID is always present in this context")
        //                 };
        //                 if rep_id == id.parse::<u8>()? {
        //                     let Some(seg_list) = representation.SegmentList.as_mut() else {
        //                         unreachable!("SegmentList is always present in this context")
        //                     };

        //                     let url = Self::path_to_source_url(&init)?;
        //                     let c2pa = base64::encode(&extract_c2pa_box(&init)?);

        //                     seg_list.Initialization = Some(Initialization {
        //                         sourceURL: Some(url),
        //                         c2pa: Some(c2pa),
        //                         ..Default::default()
        //                     });

        //                     let mut seg_urls = Vec::with_capacity(paths.len());

        //                     for path in paths {
        //                         let media = Self::path_to_source_url(path)?;
        //                         let c2pa = base64::encode(&extract_c2pa_box(path)?);

        //                         seg_urls.push(SegmentURL {
        //                             media: Some(media),
        //                             c2pa: Some(c2pa),
        //                             ..Default::default()
        //                         });
        //                     }

        //                     seg_list.segment_urls = seg_urls;
        //                 }
        //             }
        //         }
        //     }
        // }

        Ok(())
    }

    pub fn insert_media_playlist_segment_list<P>(
        &self,
        init: P,
        paths: &[PathBuf],
    ) -> Result<Option<(Vec<u8>, Url)>>
    where
        P: AsRef<Path>,
    {
        let UriInfo { rep_id, index: _ } = self.re.uri(&init)?;

        if let Some(mut entry) = self.media.get_mut(&rep_id) {
            let mut payload = Vec::new();

            let (media, url) = entry.value_mut();

            let duration = media
                .segments
                .first()
                .context("empty segment list")?
                .duration;

            let date_clones = media
                .segments
                .iter()
                .map(|s| s.program_date_time)
                .collect::<Vec<_>>();

            let mut insert = Vec::new();
            for (idx, pair) in media.segments.iter_mut().zip_longest(paths).enumerate() {
                match pair {
                    EitherOrBoth::Both(og, new) => {
                        if let Some(map) = og.map.as_mut() {
                            // replace init UUID Box data
                            map.c2pa = Some(Self::read_uuid_base64(&init)?);
                        }
                        // replace URI and UUID Box data
                        og.uri = Self::path_to_source_url(new)?;
                        og.c2pa = Some(Self::read_uuid_base64(new)?);

                        // use previous program time or create new one
                        og.program_date_time = if let Some(next) = date_clones.get(idx + 1) {
                            *next
                        } else {
                            Some(Self::now()?.into())
                        };
                    }
                    EitherOrBoth::Right(new) => {
                        // mark new Fragment for insertion (only happens once)
                        insert.push(MediaSegment {
                            uri: Self::path_to_source_url(new)?,
                            duration,
                            program_date_time: Some(Self::now()?.into()),
                            c2pa: Some(Self::read_uuid_base64(new)?),
                            ..Default::default()
                        });
                    }
                    _ => unreachable!("MediaPlaylist can't have more elements"),
                }
            }

            // insert the new Fragments
            media.segments.append(&mut insert);

            media.write_to(&mut payload)?;

            let mut vec = Vec::new();
            media.write_to(&mut vec)?;
            std::fs::write("/home/phi60110/Work/c2pa/poc-c2pa-live-demo/test.m3u8", vec)?;

            return Ok(Some((payload, url.to_owned())));
        }

        Ok(None)
    }

    /// Checks if the MPD is ready to publish with all
    /// UUID Boxes populated.
    ///
    /// Returns the serialized MPD + URL and resets the MPD
    /// for the next segments.
    pub async fn mpd_ready(&self) -> Option<(String, Url)> {
        //     let lock = self.mpd.read().await;
        //     let (mpd, url) = lock.to_owned()?;
        //     for period in mpd.periods.iter() {
        //         for adaptation in period.adaptations.iter() {
        //             for representation in adaptation.representations.iter() {
        //                 let seg_list = representation.SegmentList.as_ref()?;

        //                 seg_list.Initialization.as_ref()?.c2pa.as_ref()?;
        //                 for segment in seg_list.segment_urls.iter() {
        //                     segment.c2pa.as_ref()?;
        //                 }
        //             }
        //         }
        //     }

        //     // MPD is ready, reset and return payload
        //     let payload = mpd.to_string();

        //     // explicitly drop the lock to prevent deadlock
        //     drop(lock);
        //     self.reset_mpd().await;

        //     Some((payload, url))
        None
    }

    /// Removes all Initialization information and
    /// empties all segment URLs.
    async fn reset_mpd(&self) {
        if let Some((mpd, _)) = self.mpd.write().await.as_mut() {
            for period in mpd.periods.iter_mut() {
                for adaptation in period.adaptations.iter_mut() {
                    for representation in adaptation.representations.iter_mut() {
                        if let Some(seg_list) = representation.SegmentList.as_mut() {
                            seg_list.Initialization = None;
                            seg_list.segment_urls = Vec::new();
                        }
                    }
                }
            }
        }
    }

    fn path_to_source_url<P>(path: P) -> Result<String>
    where
        P: AsRef<Path>,
    {
        let rep = path
            .as_ref()
            .parent()
            .context("missing parent sourceURL")?
            .file_name()
            .context("missing parent file name sourceURL")?
            .to_string_lossy();
        let file = path
            .as_ref()
            .file_name()
            .context("missing file name sourceURL")?
            .to_string_lossy();

        Ok(format!("{rep}/{file}"))
    }

    fn read_uuid_base64<P>(path: P) -> Result<String>
    where
        P: AsRef<Path>,
    {
        Ok(base64::encode(&extract_c2pa_box(path)?))
    }

    fn now() -> Result<chrono::DateTime<chrono::Utc>> {
        Ok(chrono::DateTime::from_timestamp_nanos(
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as i64,
        ))
    }

    pub async fn add_rep(&self) {
        let mut lock = self.num_reps.write().await;
        *lock += 1;
    }

    pub async fn num_reps(&self) -> usize {
        *self.num_reps.read().await
    }
}
