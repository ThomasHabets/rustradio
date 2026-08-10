//! Shared UHD device ownership.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::{Complex, Error, Result};

/// A shared connection to one UHD device.
///
/// Clone this value and pass the clones to [`crate::uhd_source::UhdSource`]
/// and [`crate::uhd_sink::UhdSink`] to use one USRP for simultaneous receive
/// and transmit. Device configuration and streamer creation are serialized,
/// but the resulting RX and TX streamers run independently.
#[derive(Clone)]
pub struct UhdDevice {
    inner: Arc<Mutex<uhd::Usrp>>,
}

impl std::fmt::Debug for UhdDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UhdDevice").finish_non_exhaustive()
    }
}

impl UhdDevice {
    /// Find UHD device addresses matching `args`.
    pub fn find(args: &str) -> Result<Vec<String>> {
        uhd::Usrp::find(args).map_err(Error::from)
    }

    /// Open the UHD device matching `args`.
    ///
    /// Use an empty string for UHD's default device selection, or a UHD device
    /// address such as `"type=b200"` or `"serial=30AEB14"`.
    pub fn open(args: &str) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(uhd::Usrp::open(args)?)),
        })
    }

    /// Run additional configuration against the underlying UHD USRP.
    ///
    /// The closure is serialized with block construction. Changing device
    /// configuration after streaming has begun is supported by UHD, but the
    /// caller is responsible for coordinating changes with running blocks.
    pub fn configure<T>(
        &self,
        configure: impl FnOnce(&mut uhd::Usrp) -> uhd::Result<T>,
    ) -> Result<T> {
        let mut usrp = self.lock()?;
        configure(&mut usrp).map_err(Error::from)
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, uhd::Usrp>> {
        self.inner
            .lock()
            .map_err(|_| Error::msg("UHD device mutex is poisoned"))
    }

    pub(crate) fn rx_stream(
        &self,
        args: &uhd::StreamArgs<Complex>,
    ) -> Result<uhd::ReceiveStreamer<'static, Complex>> {
        let mut usrp = self.lock()?;
        let stream = usrp.get_rx_stream(args)?;

        // SAFETY: uhd 0.4 gives streamers the lifetime of the mutable borrow
        // used to create them, even though the streamer contains only a UHD
        // handle and PhantomData<&Usrp>. `self.inner` keeps the same Usrp at a
        // stable address, and every block stores a UhdDevice clone after its
        // streamer field, so the streamer is dropped before the last Usrp
        // owner can be dropped. The mutex serializes the mutable creation and
        // configuration calls; it is deliberately not held while streaming.
        Ok(unsafe {
            std::mem::transmute::<
                uhd::ReceiveStreamer<'_, Complex>,
                uhd::ReceiveStreamer<'static, Complex>,
            >(stream)
        })
    }

    pub(crate) fn tx_stream(
        &self,
        args: &uhd::StreamArgs<Complex>,
    ) -> Result<uhd::TransmitStreamer<'static, Complex>> {
        let mut usrp = self.lock()?;
        let stream = usrp.get_tx_stream(args)?;

        // SAFETY: This is the transmit counterpart to `rx_stream`; see its
        // safety argument. The block retains a clone of the same UhdDevice.
        Ok(unsafe {
            std::mem::transmute::<
                uhd::TransmitStreamer<'_, Complex>,
                uhd::TransmitStreamer<'static, Complex>,
            >(stream)
        })
    }
}

impl From<uhd::Error> for Error {
    fn from(source: uhd::Error) -> Self {
        let detail = uhd::last_error_message().unwrap_or_default();
        Error::device(source, format!("UHD {detail}"))
    }
}
