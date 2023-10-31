/*! Blocks for the Au file format.

The format is very simple, and is documented on
<https://en.wikipedia.org/wiki/Au_file_format>.

The benefit .au has over .wav is that .au can be written as a stream,
without seeking back to the file header to update data sizes.

It's also much simpler.
*/

use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp};
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
use rustradio::Complex;
let src = VectorSource::new(
    vec![10.0, 0.0, -20.0, 0.0, 100.0, -100.0],
);
let src_out = src.out();
let au = AuEncode::new(src_out, Encoding::PCM16, 48000, 1);
let au_out = au.out();
let sink = FileSink::new(au_out, "/dev/null", Mode::Overwrite)?;
let mut g = Graph::new();
g.add(Box::new(src));
g.add(Box::new(au));
g.add(Box::new(sink));
g.run()?;
# Ok::<(), anyhow::Error>(())
```
*/
pub struct AuEncode {
    header: Option<Vec<u8>>,
    encoding: Encoding,
    src: Streamp<Float>,
    dst: Streamp<u8>,
}

impl AuEncode {
    /// Create new Au encoder block.
    ///
    /// * `encoding`: currently only `Encoding::PCM16` is implemented.
    /// * `bitrate`: E.g. 48000,
    /// * `channels`: Currently only mono (1) is implemented.
    pub fn new(src: Streamp<Float>, encoding: Encoding, bitrate: u32, channels: u32) -> Self {
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
            dst: new_streamp(),
        }
    }
    /// Return the output stream.
    pub fn out(&self) -> Streamp<u8> {
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
            o.write(h.iter().copied());
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
        i.clear();
        if v.is_empty() {
            Ok(BlockRet::Noop)
        } else {
            o.write_slice(&v);
            Ok(BlockRet::Ok)
        }
    }
}

enum DecodeState {
    WaitingMagic,
    WaitingSize,
    WaitingHeader(usize),
    Data,
}

/// .au file decoder.
///
/// Currently only accepts a very narrow header format of PCM16, mono,
/// 44100 Hz.
pub struct AuDecode {
    src: Streamp<u8>,
    dst: Streamp<Float>,
    state: DecodeState,
}

impl AuDecode {
    /// Create new AuDecode block.
    pub fn new(src: Streamp<u8>) -> Self {
        Self {
            src,
            dst: new_streamp(),
            state: DecodeState::WaitingMagic,
        }
    }
    /// Return the output stream.
    pub fn out(&self) -> Streamp<Float> {
        self.dst.clone()
    }
}

impl Block for AuDecode {
    fn block_name(&self) -> &'static str {
        "AuDecode"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut i = self.src.lock()?;
        if i.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.lock()?;
        match self.state {
            DecodeState::WaitingMagic => {
                if i.available() < 4 {
                    return Ok(BlockRet::Noop);
                }
                let magic = i.iter().take(4).copied().collect::<Vec<_>>();
                let magic = u32::from_be_bytes(magic.try_into().unwrap());
                i.consume(4);
                assert_eq!(magic, 0x2e736e64u32);
                self.state = DecodeState::WaitingSize;
            }
            DecodeState::WaitingSize => {
                if i.available() < 4 {
                    return Ok(BlockRet::Noop);
                }
                let len = i.iter().take(4).copied().collect::<Vec<_>>();
                let len = u32::from_be_bytes(len.try_into().unwrap());
                i.consume(4);
                assert_eq!(len, 44);
                self.state = DecodeState::WaitingHeader(len as usize);
            }
            DecodeState::WaitingHeader(len) => {
                if i.available() < len {
                    return Ok(BlockRet::Noop);
                }
                let head = i.iter().take(len).copied().collect::<Vec<_>>();
                assert_eq!(
                    Encoding::PCM16 as u32,
                    u32::from_be_bytes(head[4..8].try_into().unwrap())
                );
                assert_eq!(
                    44100u32,
                    u32::from_be_bytes(head[8..12].try_into().unwrap())
                );
                assert_eq!(1u32, u32::from_be_bytes(head[12..16].try_into().unwrap()));
                self.state = DecodeState::Data;
            }
            DecodeState::Data => {
                let n = i.available() - (i.available() & 1);
                let v = i
                    .iter()
                    .take(n)
                    .copied()
                    .collect::<Vec<u8>>()
                    .chunks_exact(2)
                    .map(|chunk| {
                        let bytes = [chunk[0], chunk[1]];
                        (i16::from_be_bytes(bytes) as Float) / 32767.0
                    })
                    .collect::<Vec<Float>>();
                o.write_slice(&v);
                i.consume(n);
            }
        };
        Ok(BlockRet::Ok)
    }
}
