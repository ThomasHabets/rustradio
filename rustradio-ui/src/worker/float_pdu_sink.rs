//! A sink block that posts PDUs of float from worker to main UI thread.
use rustradio::Float;
use rustradio::block::{Block, BlockRet};
use rustradio::stream::NCReadStream;

use crate::TaggedVec;
use crate::WorkerToMain;
use crate::worker::send_message_sync;

/// Downsampling used for debugging.
const DEBUG_KEEP_1_IN_N: usize = 1;

/// A block that takes a PDU full of floats and posts it to the main UI thread.
///
/// The stream is identified by its name.
#[derive(rustradio_macros::Block)]
#[rustradio(new)]
pub struct FloatPduSink<App> {
    /// Name of the stream, for the main thread to multiplex on.
    name: String,
    #[rustradio(in)]
    src: NCReadStream<Vec<Float>>,

    // This is used for debugging only.
    #[rustradio(default)]
    skip: usize,

    #[rustradio(default)]
    _dummy: std::marker::PhantomData<App>,
}

impl<App: crate::ApplicationSpecific> Block for FloatPduSink<App> {
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        loop {
            let Some((samples, tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            self.skip += 1;
            if self.skip == DEBUG_KEEP_1_IN_N {
                send_message_sync(WorkerToMain::<App>::Floats(
                    self.name.clone(),
                    vec![TaggedVec {
                        data: samples,
                        tags,
                    }],
                ))?;
                self.skip = 0;
            }
        }
    }
}
