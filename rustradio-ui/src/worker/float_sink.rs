//! A sink block that posts the data stream from worker to main UI thread.
use rustradio::Float;
use rustradio::block::{Block, BlockRet};
use rustradio::stream::ReadStream;

use crate::worker::send_message_sync;

use crate::TaggedVec;
use crate::WorkerToMain;

/// A block that takes float data from its input and posts it to the main UI
/// thread.
///
/// The stream is identified by its name.
#[derive(rustradio_macros::Block)]
#[rustradio(new)]
pub struct FloatSink<App> {
    name: String,
    #[rustradio(in)]
    src: ReadStream<Float>,

    #[rustradio(default)]
    _dummy: std::marker::PhantomData<App>,
}

impl<App: crate::ApplicationSpecific> Block for FloatSink<App> {
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        let (input, tags) = self.src.read_buf()?;
        let ilen = input.len();
        if ilen > 0 {
            send_message_sync(WorkerToMain::<App>::Floats(
                self.name.clone(),
                vec![TaggedVec {
                    data: input.slice().to_vec(),
                    tags,
                }],
            ))?;
            input.consume(ilen);
        }
        Ok(BlockRet::WaitForStream(&self.src, 1))
    }
}
