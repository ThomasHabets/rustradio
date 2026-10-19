//! Metadata shared by the endpoints of a stream.
use std::sync::Mutex;

use crate::{Error, Result};

#[derive(Debug)]
struct Info {
    name: String,
    sample_rate: Option<f64>,
}

/// Metadata shared by a stream's reader and writer through their `Arc`-owned
/// storage. Owning one endpoint does not give exclusive access to that storage,
/// so setters need interior mutability to update metadata visible to both ends.
/// The mutex keeps these updates thread-safe and independent of the locks used
/// for sample transfer; processing samples does not acquire this mutex.
#[derive(Debug)]
pub(crate) struct Metadata(Mutex<Info>);

impl Metadata {
    pub fn new(id: usize) -> Self {
        Self(Mutex::new(Info {
            name: format!("stream-{id}"),
            sample_rate: None,
        }))
    }

    pub fn name(&self) -> String {
        self.0.lock().unwrap().name.clone()
    }

    pub fn sample_rate(&self) -> Option<f64> {
        self.0.lock().unwrap().sample_rate
    }

    pub fn set_name(&self, name: impl Into<String>) {
        let name = name.into();
        if !name.is_empty() {
            self.0.lock().unwrap().name = name;
        }
    }

    pub fn set_sample_rate(&self, rate: f64) -> Result<()> {
        if !rate.is_finite() || rate <= 0.0 {
            return Err(Error::msg("sample rate must be finite and positive"));
        }
        self.0.lock().unwrap().sample_rate = Some(rate);
        Ok(())
    }
}
