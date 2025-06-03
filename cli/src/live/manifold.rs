use std::time::Duration;

use anyhow::{Context, Result};
use dashmap::DashMap;
use serde::Serialize;
use tokio_retry::{strategy::FibonacciBackoff, Retry};

#[derive(Debug, Serialize, Clone)]
pub struct EventPayload {
    /// optional anchor point base64 encoded
    #[serde(rename = "anchorPoint")]
    anchor_point: Option<Vec<u8>>,

    /// rolling hash base64 encoded
    #[serde(rename = "rollingHash")]
    rolling_hash: Vec<u8>,

    /// starts at 2 (MPD + MediaPlaylist)
    ///     - each read decrements
    ///     - at 0 removed from map
    #[serde(skip)]
    count: usize,
}

impl EventPayload {
    pub fn new(rh: &[u8], ap: &Option<Vec<u8>>) -> Self {
        Self {
            anchor_point: ap.to_owned(),
            rolling_hash: rh.to_owned(),
            count: 1, // TODO change to 2 when including HLS
        }
    }
}

#[derive(Default)]
pub struct Manifold {
    map: DashMap<String, EventPayload>,
}

impl Manifold {
    pub fn insert(&self, rep: &str, event: EventPayload) {
        self.map.insert(rep.to_string(), event);
    }

    pub async fn get(&self, rep: &str) -> Result<EventPayload> {
        let mut lock = self.map.get_mut(rep).context("missing value")?;
        lock.count -= 1;

        let clone = lock.clone();

        if lock.count == 0 {
            drop(lock);
            self.remove(rep);
        }

        Ok(clone)
    }

    pub fn remove(&self, rep: &str) {
        self.map.remove(rep);
    }

    pub async fn get_json(&self, rep: &str) -> Result<Vec<u8>> {
        let strategy = FibonacciBackoff::from_millis(100).max_delay(Duration::from_millis(500));
        let res = Retry::spawn(strategy, || self.get(rep)).await?;

        Ok(serde_json::to_vec(&res)?)
    }
}
