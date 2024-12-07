//! Decode RTL-SDR's byte based format into Complex I/Q.
use anyhow::Result;

use crate::block::{Block, BlockRet};
use crate::stream::Streamp;
use crate::{Complex, Error, Float};

/// Decode RTL-SDR's byte based format into Complex I/Q.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, out)]
pub struct RtlSdrDecode {
    #[rustradio(in)]
    src: Streamp<u8>,
    #[rustradio(out)]
    dst: Streamp<Complex>,
}

impl Block for RtlSdrDecode {
    fn work(&mut self) -> Result<BlockRet, Error> {
        // TODO: handle tags.
        let (input, _tags) = self.src.read_buf()?;
        let isamples = input.len() - input.len() % 2;
        let osamples = isamples / 2;
        if isamples == 0 || osamples == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut out = self.dst.write_buf()?;

        // TODO: needless copy.
        out.fill_from_iter((0..isamples).step_by(2).map(|e| {
            Complex::new(
                ((input[e] as Float) - 127.0) * 0.008,
                ((input[e + 1] as Float) - 127.0) * 0.008,
            )
        }));
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
        let mut src = VectorSource::new(vec![]);
        assert_eq!(src.work()?, BlockRet::EOF);
        let mut dec = RtlSdrDecode::new(src.out());
        assert_eq!(dec.work()?, BlockRet::Noop);
        let os = dec.out();
        let (res, _) = os.read_buf()?;
        assert_eq!(res.len(), 0);
        Ok(())
    }

    #[test]
    fn some_input() -> crate::Result<()> {
        let mut src = VectorSource::new(vec![0, 10, 20, 10, 0, 13]);
        assert_eq!(src.work()?, BlockRet::Ok);
        assert_eq!(src.work()?, BlockRet::EOF);
        let mut dec = RtlSdrDecode::new(src.out());
        assert_eq!(dec.work()?, BlockRet::Ok);
        let os = dec.out();
        let (res, _) = os.read_buf()?;
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
        let mut src = VectorSource::new(vec![0, 10, 20, 10, 0]);
        assert_eq!(src.work()?, BlockRet::Ok);
        assert_eq!(src.work()?, BlockRet::EOF);
        let mut dec = RtlSdrDecode::new(src.out());
        assert_eq!(dec.work()?, BlockRet::Ok);
        let os = dec.out();
        let (res, _) = os.read_buf()?;
        assert_eq!(res.len(), 2);
        Ok(())
    }
}
