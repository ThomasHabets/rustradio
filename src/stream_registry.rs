//! Process-wide inventory of live stream metadata.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, Weak};

use crate::stream_metadata::Metadata;

/// Snapshot of a live process stream's metadata.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct StreamInfo {
    /// Process-wide stream identity, shared by both endpoints.
    pub id: usize,
    /// Explicit stream name; `None` uses the name derived from `id`.
    pub name: Option<String>,
    /// Rust sample or message type.
    pub sample_type: String,
    /// Whether items are individually owned messages rather than copying samples.
    pub messages: bool,
    /// Samples (or messages) per second, when known.
    pub sample_rate_hz: Option<f64>,
    /// The producer and any outstanding write window have finished.
    /// Buffered items may still remain for the consumer to read.
    pub ended: bool,
}

impl StreamInfo {
    /// Return the explicit name, or `stream-{id}` for an unnamed stream.
    #[must_use]
    pub fn name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("stream-{}", self.id))
    }
}

pub(crate) type Registry = HashMap<usize, Weak<Metadata>>;
pub(crate) fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// List live process streams, ordered by name, including custom item types.
///
/// Both endpoints and their outstanding buffer windows share one registration.
/// Registrations hold weak metadata references and do not keep buffers alive or
/// affect stream EOF. Expired registrations are removed when listing or creating
/// streams. Returned snapshots do not retain their stream's registration.
#[must_use]
pub fn streams() -> Vec<StreamInfo> {
    let mut registry = registry().lock().unwrap();
    registry.retain(|_, metadata| metadata.strong_count() > 0);
    let mut entries: Vec<_> = registry
        .values()
        .filter_map(Weak::upgrade)
        .map(|metadata| metadata.info())
        .collect();
    entries.sort_by_cached_key(StreamInfo::name);
    entries
}
