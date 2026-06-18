//! FFT stream.
//!
//! Takes a stream of data, runs an FFT on it, and outputs it as a stream.
//! The consumer of the stream needs to know what the FFT size is, or it won't
//! be able to make sense of it.
use crate::Result;
use rustfft::FftPlanner;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, Tag, TagValue, WriteStream};
use crate::{Complex, Float};

/// Boolean tag marking each FFT frame in the output stream.
///
/// `true` is attached to the first FFT bin and `false` is attached to the last
/// FFT bin. `StreamToPdu` users should pass `tail = 1` to include the sample
/// carrying the end tag.
pub const TAG_FRAME: &str = "FftStream::frame";

/// Tag with the FFT size.
pub const TAG_FRAME_SIZE: &str = "FftStream::size";

/// Takes a stream of data, runs an FFT on it, and outputs it as a stream.
/// The consumer of the stream needs to know what the FFT size is, or it won't
/// be able to make sense of it.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct FftStream {
    size: usize,
    fft: std::sync::Arc<dyn rustfft::Fft<Float>>,
    threaded: bool,
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl FftStream {
    /// Create a new `FftStream`.
    #[must_use]
    pub fn new(src: ReadStream<Complex>, size: usize) -> (Self, ReadStream<Complex>) {
        assert_ne!(size, 0, "FFT size must be nonzero");
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(size);
        let (dst, dr) = crate::stream::new_stream();
        (
            Self {
                size,
                fft,
                src,
                dst,
                threaded: false,
            },
            dr,
        )
    }
    /// Turn on or off Rayon multithreading.
    ///
    /// Initial benchmarks seem to indicate that this does not help. Maybe with
    /// bigger than default stream buffers for more concurrency.
    pub fn threaded(&mut self, onoff: bool) {
        self.threaded = onoff;
    }
}

impl Block for FftStream {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        let (input, _tags) = self.src.read_buf()?;
        let ii = input.slice();
        if ii.len() < self.size {
            return Ok(BlockRet::WaitForStream(&self.src, self.size));
        }
        let mut o = self.dst.write_buf()?;
        let oo = o.slice();
        if oo.len() < self.size {
            return Ok(BlockRet::WaitForStream(&self.dst, self.size));
        }
        let len = std::cmp::min(ii.len(), oo.len());
        let len = len - (len % self.size);
        oo[..len].copy_from_slice(&ii[..len]);

        // It would be nice to use fft.process_outofplace_with_scratch(), but it
        // requires input also be scratch space, and therefore mutable.
        if self.threaded {
            use rayon::prelude::*;
            oo.par_chunks_exact_mut(self.size).for_each(|chunk| {
                self.fft.process(chunk);
            });
        } else {
            oo.chunks_exact_mut(self.size).for_each(|chunk| {
                self.fft.process(chunk);
            });
        }
        let mut tags = Vec::with_capacity((len / self.size) * 2);
        for pos in (0..len).step_by(self.size) {
            tags.push(Tag::new(
                pos,
                TAG_FRAME_SIZE,
                TagValue::U64(u64::try_from(self.size)?),
            ));
            tags.push(Tag::new(pos, TAG_FRAME, TagValue::Bool(true)));
            tags.push(Tag::new(
                pos + self.size - 1,
                TAG_FRAME,
                TagValue::Bool(false),
            ));
        }

        input.consume(len);
        o.produce(len, &tags);
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::StreamToPdu;

    #[test]
    fn adds_frame_tags() -> Result<()> {
        let src = ReadStream::from_slice(&[Complex::default(); 8]);
        let (mut fft, out) = FftStream::new(src, 4);

        assert!(matches!(fft.work()?, BlockRet::Again));
        let (buf, tags) = out.read_buf()?;

        assert_eq!(buf.len(), 8);
        assert_eq!(
            tags,
            [
                Tag::new(0, TAG_FRAME_SIZE, TagValue::U64(4)),
                Tag::new(0, TAG_FRAME, TagValue::Bool(true)),
                Tag::new(3, TAG_FRAME, TagValue::Bool(false)),
                Tag::new(4, TAG_FRAME_SIZE, TagValue::U64(4)),
                Tag::new(4, TAG_FRAME, TagValue::Bool(true)),
                Tag::new(7, TAG_FRAME, TagValue::Bool(false)),
            ]
        );
        Ok(())
    }

    #[test]
    fn output_can_be_batched_by_stream_to_pdu() -> Result<()> {
        let src = ReadStream::from_slice(&[Complex::default(); 8]);
        let (mut fft, fft_out) = FftStream::new(src, 4);
        let (mut to_pdu, pdu_out) = StreamToPdu::new(fft_out, TAG_FRAME, 4, 1);

        assert!(matches!(fft.work()?, BlockRet::Again));
        assert!(matches!(to_pdu.work()?, BlockRet::WaitForStream(_, 1)));

        let (first, tags) = pdu_out.pop().unwrap();
        assert_eq!(first, vec![Complex::default(); 4]);
        assert_eq!(tags, &[Tag::new(0, "FftStream::size", TagValue::U64(4)),]);

        let (second, tags) = pdu_out.pop().unwrap();
        assert_eq!(second, vec![Complex::default(); 4]);
        assert_eq!(tags, &[Tag::new(0, "FftStream::size", TagValue::U64(4)),]);
        assert!(pdu_out.pop().is_none());
        Ok(())
    }
}
/* vim: textwidth=80
 */
