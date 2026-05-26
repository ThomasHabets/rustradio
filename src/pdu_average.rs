//! Average batches of float PDUs.

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream};
use crate::{Error, Float, Result};

/// Average every `n` float PDUs into one output PDU.
///
/// This averages corresponding positions across the input PDUs. All PDUs in a
/// batch must have the same length. Input tags are discarded.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct PduAverage {
    n: usize,

    #[rustradio(in)]
    src: NCReadStream<Vec<Float>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<Float>>,

    #[rustradio(default)]
    acc: Vec<Float>,
    #[rustradio(default)]
    count: usize,
}

impl PduAverage {
    fn add_pdu(&mut self, pdu: Vec<Float>) -> Result<()> {
        if self.count == 0 {
            self.acc = pdu;
        } else {
            if pdu.len() != self.acc.len() {
                let want = self.acc.len();
                self.acc.clear();
                self.count = 0;
                return Err(Error::msg(format!(
                    "PduAverage got PDU length {}, but current batch length is {want}",
                    pdu.len()
                )));
            }
            for (acc, sample) in self.acc.iter_mut().zip(pdu) {
                *acc += sample;
            }
        }
        self.count += 1;
        Ok(())
    }

    fn finish_batch(&mut self) -> Vec<Float> {
        debug_assert_eq!(self.count, self.n);
        let mut ret = std::mem::take(&mut self.acc);
        let scale = 1.0 / self.n as Float;
        for sample in &mut ret {
            *sample *= scale;
        }
        self.count = 0;
        ret
    }
}

impl Block for PduAverage {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        if self.n == 0 {
            return Err(Error::msg("PduAverage with n=0 is invalid"));
        }
        loop {
            if self.dst.remaining() == 0 {
                return Ok(BlockRet::WaitForStream(&self.dst, 1));
            }
            let Some((pdu, _tags)) = self.src.pop() else {
                return Ok(BlockRet::WaitForStream(&self.src, 1));
            };
            self.add_pdu(pdu)?;
            if self.count == self.n {
                let ret = self.finish_batch();
                self.dst.push(ret, &[]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::new_nocopy_stream;

    #[test]
    fn waits_for_full_batch() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut avg, out) = PduAverage::new(rx, 2);

        tx.push(vec![1.0, 3.0], &[]);
        assert!(matches!(avg.work()?, BlockRet::WaitForStream(_, 1)));
        assert!(out.pop().is_none());

        tx.push(vec![3.0, 5.0], &[]);
        assert!(matches!(avg.work()?, BlockRet::WaitForStream(_, 1)));
        let (pdu, tags) = out.pop().unwrap();
        assert_eq!(pdu, vec![2.0, 4.0]);
        assert_eq!(tags, &[]);
        Ok(())
    }

    #[test]
    fn averages_full_batches() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut avg, out) = PduAverage::new(rx, 3);

        tx.push(vec![1.0, 2.0, 3.0], &[]);
        tx.push(vec![4.0, 5.0, 6.0], &[]);
        tx.push(vec![7.0, 8.0, 9.0], &[]);

        assert!(matches!(avg.work()?, BlockRet::WaitForStream(_, 1)));
        let (pdu, tags) = out.pop().unwrap();
        assert_eq!(pdu, vec![4.0, 5.0, 6.0]);
        assert_eq!(tags, &[]);
        Ok(())
    }

    #[test]
    fn rejects_mismatched_lengths() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut avg, out) = PduAverage::new(rx, 2);

        tx.push(vec![1.0, 2.0], &[]);
        tx.push(vec![3.0], &[]);

        let err = avg.work().unwrap_err();
        assert!(err.to_string().contains("PDU length"));
        assert!(out.pop().is_none());
        Ok(())
    }
}
