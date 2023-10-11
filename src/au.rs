/*! Blocks for the Au file format.

The format is very simple, and is documented on
<https://en.wikipedia.org/wiki/Au_file_format>.

The benefit .au has over .wav is that .au can be written as a stream,
without seeking back to the file header to update data sizes.

It's also much simpler.
*/
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::Stream;
use crate::{Error, Float};

/// Au support several encodings. This code currently only one.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Encoding {
    /// 16 bit linear PCM.
    PCM16 = 3,
}

/** Au encoder block.

This block takes a stream of floats between -1 and 1, and writes them
as the bytes of an .au file.

```
use rustradio::graph::Graph;
use rustradio::blocks::{AuEncode, VectorSource, FileSink};
use rustradio::au::Encoding;
use rustradio::file_sink::Mode;
use rustradio::stream::StreamType;
use rustradio::Complex;
let mut g = Graph::new();
let src = g.add(Box::new(VectorSource::new(
    vec![10.0, 0.0, -20.0, 0.0, 100.0, -100.0],
    false,
)));
let au = g.add(Box::new(AuEncode::new(Encoding::PCM16, 48000, 1)));;
let sink = g.add(Box::new(FileSink::<u8>::new("/dev/null", Mode::Overwrite)?));
g.connect(StreamType::new_float(), src, 0, au, 0);
g.connect(StreamType::new_u8(), au, 0, sink, 0);
g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
pub struct AuEncode {
    header: Option<Vec<u8>>,
    encoding: Encoding,
    src: Arc<Mutex<Stream<Float>>>,
    dst: Arc<Mutex<Stream<u8>>>,
}

impl AuEncode {
    /// Create new Au encoder block.
    ///
    /// * `encoding`: currently only `Encoding::PCM16` is implemented.
    /// * `bitrate`: E.g. 48000,
    /// * `channels`: Currently only mono (1) is implemented.
    pub fn new(
        src: Arc<Mutex<Stream<Float>>>,
        encoding: Encoding,
        bitrate: u32,
        channels: u32,
    ) -> Self {
        assert_eq!(channels, 1, "only mono supported at the moment");
        let mut v = Vec::with_capacity(28);

        // Magic
        v.extend(0x2e736e64u32.to_be_bytes());

        // Data offset.
        v.extend(28u32.to_be_bytes());

        // Size, or all ones if unknown.
        v.extend(0xffffffffu32.to_be_bytes());

        // Mode.
        v.extend((encoding as u32).to_be_bytes());

        // Bitrate.
        v.extend(bitrate.to_be_bytes());

        // Channels.
        v.extend(channels.to_be_bytes());

        // Minimum annotation field.
        v.extend(&[0, 0, 0, 0]);

        Self {
            header: Some(v),
            encoding,
            src,
            dst: Arc::new(Mutex::new(Stream::<u8>::new())),
        }
    }
    pub fn out(&self) -> Arc<Mutex<Stream<u8>>> {
        self.dst.clone()
    }
}

impl Block for AuEncode {
    fn block_name(&self) -> &'static str {
        "AuEncode"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut o = self.dst.lock().unwrap();
        if let Some(h) = &self.header {
            o.write_slice(h);
            self.header = None;
        }

        assert_eq!(self.encoding, Encoding::PCM16);
        type S = i16;
        let scale = S::MAX as Float;

        let mut i = self.src.lock().unwrap();
        let mut v = Vec::with_capacity(i.available() * std::mem::size_of::<S>());
        i.iter().for_each(|x: &Float| {
            v.extend(((*x * scale) as S).to_be_bytes());
        });
        o.write_slice(&v);
        i.clear();
        Ok(BlockRet::Ok)
    }
}
