//! Decode RTL-SDR's byte based format into Complex I/Q.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Error, Float};

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
    fn work(&mut self) -> Result<BlockRet, Error> {
        // TODO: handle tags.
        let (input, _tags) = self.src.read_buf()?;
        let isamples = input.len() & !1;
        let mut out = self.dst.write_buf()?;
        if isamples == 0 {
            return Ok(BlockRet::Noop);
        }
        let isamples = std::cmp::min(isamples, out.len() * 2);
        let osamples = isamples / 2;
        if osamples == 0 {
            return Ok(BlockRet::OutputFull);
        }

        out.fill_from_iter(
            input
                .slice()
                .chunks_exact(2)
                .map(|e| ((e[0] as Float), (e[1] as Float)))
                .map(|(a, b)| Complex::new((a - 127.0) * 0.008, (b - 127.0) * 0.008)),
        );
        input.consume(isamples);
        out.produce(osamples, &[]);
        Ok(BlockRet::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;
    use crate::Complex;

    #[test]
    fn empty() -> crate::Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![]);
        assert_eq!(src.work()?, BlockRet::EOF);
        let (mut dec, dec_out) = RtlSdrDecode::new(src_out);
        assert_eq!(dec.work()?, BlockRet::Noop);
        let (res, _) = dec_out.read_buf()?;
        assert_eq!(res.len(), 0);
        Ok(())
    }

    #[test]
    fn some_input() -> crate::Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![0, 10, 20, 10, 0, 13]);
        assert_eq!(src.work()?, BlockRet::Ok);
        assert_eq!(src.work()?, BlockRet::EOF);
        let (mut dec, dec_out) = RtlSdrDecode::new(src_out);
        assert_eq!(dec.work()?, BlockRet::Ok);
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
        assert_eq!(src.work()?, BlockRet::Ok);
        assert_eq!(src.work()?, BlockRet::EOF);
        let (mut dec, dec_out) = RtlSdrDecode::new(src_out);
        assert_eq!(dec.work()?, BlockRet::Ok);
        let (res, _) = dec_out.read_buf()?;
        assert_eq!(res.len(), 2);
        Ok(())
    }

    #[test]
    fn overflow() -> crate::Result<()> {
        // Input is pairs of bytes. Output is complex, meaning a 4x increase. That won't fit.
        let (mut src, src_out) = VectorSource::new(vec![0; crate::stream::DEFAULT_STREAM_SIZE]);
        assert_eq!(src.work()?, BlockRet::Ok);
        assert_eq!(src.work()?, BlockRet::EOF);
        let (mut dec, dec_out) = RtlSdrDecode::new(src_out);
        for n in 0..4 {
            eprintln!("loop iter: {n}");
            assert_eq!(dec.work()?, BlockRet::Ok);
            let (res, _) = dec_out.read_buf()?;
            assert_eq!(res.len(), crate::stream::DEFAULT_STREAM_SIZE / 8);
            res.consume(crate::stream::DEFAULT_STREAM_SIZE / 8);
        }
        // Finally there's no more input bytes to process.
        assert_eq!(dec.work()?, BlockRet::Noop);
        Ok(())
    }
}
