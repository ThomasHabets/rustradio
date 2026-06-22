//! Encode Complex I/Q into RTL-SDR's byte based format.
use crate::Result;

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Float};

const RTLSDR_OFFSET: Float = 127.0;
const RTLSDR_SCALE: Float = 0.008;

/// Encode Complex I/Q into RTL-SDR's byte based format.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct RtlSdrEncode {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
}

fn encode_sample(sample: Float) -> u8 {
    ((sample / RTLSDR_SCALE) + RTLSDR_OFFSET)
        .round()
        .clamp(0.0, 255.0) as u8
}

impl Block for RtlSdrEncode {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        loop {
            // TODO: handle tags.
            let (input, _tags) = self.src.read_buf()?;
            if input.is_empty() {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            }
            let mut out = self.dst.write_buf()?;
            if out.len() < 2 {
                return Ok(BlockRet::WaitForStream(&self.dst, 2));
            }
            let isamples = std::cmp::min(input.len(), out.len() / 2);
            let osamples = isamples * 2;
            assert_ne!(isamples, 0);

            out.fill_from_iter(
                input
                    .slice()
                    .iter()
                    .take(isamples)
                    .flat_map(|sample| [encode_sample(sample.re), encode_sample(sample.im)]),
            );
            input.consume(isamples);
            out.produce(osamples, &[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::VectorSource;

    #[test]
    fn empty() -> crate::Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![]);
        let r = src.work()?;
        assert!(matches![r, BlockRet::EOF], "Want EOF, got {r:?}");
        let (mut enc, enc_out) = RtlSdrEncode::new(src_out);
        assert!(matches![enc.work()?, BlockRet::WaitForStream(_, _)]);
        let (res, _) = enc_out.read_buf()?;
        assert_eq!(res.len(), 0);
        Ok(())
    }

    #[test]
    fn some_input() -> crate::Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![
            Complex::new(-1.016, -0.93600005),
            Complex::new(-0.85600007, -0.93600005),
            Complex::new(-1.016, -0.91200006),
        ]);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut enc, enc_out) = RtlSdrEncode::new(src_out);
        assert!(matches![enc.work()?, BlockRet::WaitForStream(_, _)]);
        let (res, _) = enc_out.read_buf()?;
        assert_eq!(res.slice(), &[0, 10, 20, 10, 0, 13]);
        Ok(())
    }

    #[test]
    fn clips_to_byte_range() -> crate::Result<()> {
        let (mut src, src_out) = VectorSource::new(vec![Complex::new(-10.0, 10.0)]);
        assert!(matches![src.work()?, BlockRet::EOF]);
        let (mut enc, enc_out) = RtlSdrEncode::new(src_out);
        assert!(matches![enc.work()?, BlockRet::WaitForStream(_, _)]);
        let (res, _) = enc_out.read_buf()?;
        assert_eq!(res.slice(), &[0, 255]);
        Ok(())
    }
}
