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
let sink = FileSink::new(au_out, "/dev/null".into(), Mode::Overwrite)?;
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
    fn block_name(&self) -> &str {
        "AuEncode"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut o = self.dst.write_buf()?;
        if let Some(h) = &self.header {
            let n = std::cmp::min(h.len(), o.len());
            o.fill_from_slice(&h[..n]);
            o.produce(n, &[]);
            self.header.as_mut().unwrap().drain(0..n);
            if self.header.as_ref().unwrap().is_empty() {
                self.header = None;
            }
            return Ok(BlockRet::Ok);
        }

        assert_eq!(self.encoding, Encoding::PCM16);
        type S = i16;
        let scale = S::MAX as Float;
        let ss = std::mem::size_of::<S>();

        let (i, _tags) = self.src.read_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let n = std::cmp::min(i.len(), o.len() / ss);
        if n == 0 {
            return Ok(BlockRet::Ok);
        }

        for j in 0..n {
            let val = (i.slice()[j] * scale) as S;
            o.slice()[j * ss..(j + 1) * ss].clone_from_slice(&val.to_be_bytes());
        }
        i.consume(n);
        o.produce(n * ss, &[]);
        Ok(BlockRet::Ok)
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
    bitrate: u32,
}

impl AuDecode {
    /// Create new AuDecode block.
    pub fn new(src: Streamp<u8>, bitrate: u32) -> Self {
        Self {
            src,
            bitrate,
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
    fn block_name(&self) -> &str {
        "AuDecode"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, _tags) = self.src.read_buf()?;
        if i.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.write_buf()?;
        match self.state {
            DecodeState::WaitingMagic => {
                if i.len() < 4 {
                    return Ok(BlockRet::Noop);
                }
                let magic = i.iter().take(4).copied().collect::<Vec<_>>();
                let magic = u32::from_be_bytes(magic.try_into().unwrap());
                i.consume(4);
                assert_eq!(magic, 0x2e736e64u32);
                self.state = DecodeState::WaitingSize;
            }
            DecodeState::WaitingSize => {
                if i.len() < 4 {
                    return Ok(BlockRet::Noop);
                }
                let data_offset = i.iter().take(4).copied().collect::<Vec<_>>();
                let data_offset = u32::from_be_bytes(data_offset.try_into().unwrap());
                i.consume(4);
                self.state = DecodeState::WaitingHeader(data_offset as usize);
            }
            DecodeState::WaitingHeader(data_offset) => {
                let header_rest_len = data_offset - 8;
                if i.len() < header_rest_len {
                    return Ok(BlockRet::Noop);
                }
                let head = i.iter().take(header_rest_len).copied().collect::<Vec<_>>();
                assert_eq!(
                    Encoding::PCM16 as u32,
                    u32::from_be_bytes(head[4..8].try_into().unwrap())
                );
                assert_eq!(
                    self.bitrate,
                    u32::from_be_bytes(head[8..12].try_into().unwrap())
                );
                assert_eq!(1u32, u32::from_be_bytes(head[12..16].try_into().unwrap()));
                self.state = DecodeState::Data;
            }
            DecodeState::Data => {
                let n = std::cmp::min(i.len(), o.len() * 2); // Bytes.
                let n = n - (n & 1);
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
                o.fill_from_iter(v);
                o.produce(n / 2, &[]);
                i.consume(n);
            }
        };
        Ok(BlockRet::Ok)
    }
}
