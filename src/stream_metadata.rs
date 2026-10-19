//! Metadata shared by the endpoints of a stream.
use std::sync::{Arc, Mutex};

use crate::stream_registry::{StreamInfo, registry};

/// Metadata shared by a stream's reader and writer through their `Arc`-owned
/// storage. Owning one endpoint does not give exclusive access to that storage,
/// so setters need interior mutability to update metadata visible to both ends.
/// The mutex keeps these updates thread-safe and independent of the locks used
/// for sample transfer; ordinary sample transfer does not acquire this mutex.
#[derive(Debug)]
pub(crate) struct Metadata {
    pub id: usize,
    info: Mutex<StreamInfo>,
}
impl Metadata {
    pub fn new<T>(id: usize, messages: bool) -> Arc<Self> {
        let mut registry = registry().lock().unwrap();
        registry.retain(|_, m| m.strong_count() > 0);
        let base = format!("stream-{id}");
        let mut name = base.clone();
        let mut suffix = 0;
        while registry
            .values()
            .filter_map(|m| m.upgrade())
            .any(|m| m.name() == name)
        {
            suffix += 1;
            name = format!("{base}-{suffix}");
        }
        let ret = Arc::new(Self {
            id,
            info: Mutex::new(StreamInfo {
                name,
                named: false,
                sample_type: std::any::type_name::<T>().into(),
                messages,
                sample_rate_hz: None,
                ended: false,
            }),
        });
        registry.insert(id, Arc::downgrade(&ret));
        ret
    }
    pub fn info(&self) -> StreamInfo {
        self.info.lock().unwrap().clone()
    }
    pub fn name(&self) -> String {
        self.info.lock().unwrap().name.clone()
    }
    pub fn set_name(&self, requested: impl Into<String>) {
        let requested = requested.into();
        if requested.is_empty() {
            return;
        }
        let registry = registry().lock().unwrap();
        let occupied = |name: &str| {
            registry.values().filter_map(|m| m.upgrade()).any(|m| {
                if std::ptr::eq(m.as_ref(), self) {
                    return false;
                }
                let other = m.info();
                other.name == name
            })
        };
        let mut name = requested.clone();
        let mut suffix = 0;
        while occupied(&name) {
            suffix += 1;
            name = if suffix == 1 {
                format!("{requested}-{}", self.id)
            } else {
                format!("{requested}-{}-{suffix}", self.id)
            };
        }
        let mut info = self.info.lock().unwrap();
        info.name = name;
        info.named = true;
    }
    pub fn sample_rate(&self) -> Option<f64> {
        self.info.lock().unwrap().sample_rate_hz
    }
    pub fn set_sample_rate(&self, rate: f64) -> crate::Result<()> {
        if !rate.is_finite() || rate <= 0.0 {
            return Err(crate::Error::msg("sample rate must be finite and positive"));
        }
        self.info.lock().unwrap().sample_rate_hz = Some(rate);
        Ok(())
    }
    /// Mark completion once the producer and any outstanding write window close.
    pub fn set_ended(&self) {
        self.info.lock().unwrap_or_else(|p| p.into_inner()).ended = true;
    }

    #[cfg(test)]
    pub fn ended(&self) -> bool {
        self.info.lock().unwrap().ended
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_disambiguate_explicit_names() {
        let first_id = crate::NEXT_STREAM_ID.fetch_add(2, std::sync::atomic::Ordering::Relaxed);
        let second_id = first_id + 1;
        let first = Metadata::new::<u8>(first_id, false);
        first.set_name(format!("stream-{second_id}"));
        let second = Metadata::new::<u8>(second_id, false);
        assert_ne!(first.name(), second.name());
        assert!(second.name().starts_with(&format!("stream-{second_id}-")));
    }
}
