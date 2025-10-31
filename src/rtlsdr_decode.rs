//! Decode RTL-SDR's byte based format into Complex I/Q.
use crate::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

/// Decode RTL-SDR's byte based format into Complex I/Q.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct RtlSdrDecode {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<Complex>,
}

impl Block for RtlSdrDecode {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        // TODO: handle tags.
        let (input, _tags) = self.src.read_buf()?;
        let isamples = input.len() & !1;
        if isamples == 0 {
            return Ok(BlockRet::WaitForStream(&self.src, 2));
        }
        let mut out = self.dst.write_buf()?;
        if out.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let isamples = std::cmp::min(isamples, out.len() * 2);
        let osamples = isamples / 2;
        assert_ne!(osamples, 0);

        out.fill_from_iter(
            input
                .slice()
                .chunks_exact(2)
                .map(|e| (Float::from(e[0]), Float::from(e[1])))
                .map(|(a, b)| Complex::new((a - 127.0) * 0.008, (b - 127.0) * 0.008)),
        );
        input.consume(isamples);
        out.produce(osamples, &[]);
        Ok(BlockRet::Again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Complex;
    use crate::blocks::VectorSource;

    #[test]
    fn empty() -> crate::Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![]);
        let r = src.work()?;
        assert!(matches![r, BlockRet::EOF], "Want EOF, got {r:?}");
        let (mut dec, dec_out) = RtlSdrDecode::new(src_out);
        assert!(matches![dec.work()?, BlockRet::WaitForStream(_, _)]);
        let (res, _) = dec_out.read_buf()?;
        assert_eq!(res.len(), 0);
        Ok(())
    }

    #[test]
    fn some_input() -> crate::Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![0, 10, 20, 10, 0, 13]);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut dec, dec_out) = RtlSdrDecode::new(src_out);
        assert!(matches![dec.work()?, BlockRet::Again]);
        let (res, _) = dec_out.read_buf()?;
        // Probably this should compare close to, but not equal.
        assert_eq!(
            res.slice(),
            &[
                Complex {
                    re: -1.016,
                    im: -0.93600005
                },
                Complex {
                    re: -0.85600007,
                    im: -0.93600005
                },
                Complex {
                    re: -1.016,
                    im: -0.91200006
                }
            ]
        );
        Ok(())
    }

    #[test]
    fn uneven() -> crate::Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![0, 10, 20, 10, 0]);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut dec, dec_out) = RtlSdrDecode::new(src_out);
        assert!(matches![dec.work()?, BlockRet::Again]);
        let (res, _) = dec_out.read_buf()?;
        assert_eq!(res.len(), 2);
        Ok(())
    }

    #[test]
    fn overflow() -> crate::Result<()> {
        // Input is pairs of bytes. Output is complex, meaning a 4x increase. That won't fit.
        let (mut src, src_out) = VectorSource::new(vec![0; crate::stream::DEFAULT_STREAM_SIZE]);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut dec, dec_out) = RtlSdrDecode::new(src_out);
        for n in 0..4 {
            eprintln!("loop iter: {n}");
            assert!(matches![dec.work()?, BlockRet::Again]);
            let (res, _) = dec_out.read_buf()?;
            assert_eq!(res.len(), crate::stream::DEFAULT_STREAM_SIZE / 8);
            res.consume(crate::stream::DEFAULT_STREAM_SIZE / 8);
        }
        // Finally there's no more input bytes to process.
        assert!(matches![dec.work()?, BlockRet::WaitForStream(_, _)]);
        Ok(())
    }
}
