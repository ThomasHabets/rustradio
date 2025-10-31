//! NRZI — Non return to zero
use crate::stream::{ReadStream, WriteStream};

/// NRZI decoder.
///
/// <https://en.wikipedia.org/wiki/Non-return-to-zero>
///
/// The same effect as `NrziDecode` can be had by doing:
///
/// ```text
/// let (prev, b) = blockchain![g, prev, Tee::new(prev)];
/// let prev = blockchain![
///     g,
///     prev,
///     Delay::new(prev, 1),
///     Xor::new(delay, b),
///     XorConst::new(prev, 1u8),
/// ];
/// ```
///
/// "NRZI" is actually ambiguous as to which is zero and which is
/// one. This code is going with NRZI-S, meaning a toggle is zero, and
/// constant is one, because that's what done by AX.25, both 1200bps Bell
/// 202, and 9600 G3RUH.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct NrziDecode {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    #[rustradio(default)]
    last: u8,
}

impl NrziDecode {
    fn process_sync(&mut self, a: u8) -> u8 {
        let tmp = self.last;
        self.last = a;
        1 ^ a ^ tmp
    }
}

/// NRZI encoder.
///
/// <https://en.wikipedia.org/wiki/Non-return-to-zero>
///
/// "NRZI" is actually ambiguous as to which is zero and which is
/// one. This code is going with NRZI-S, meaning a toggle is zero, and
/// constant is one, because that's what done by AX.25, both 1200bps Bell
/// 202, and 9600 G3RUH.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new, sync)]
pub struct NrziEncode {
    #[rustradio(in)]
    src: ReadStream<u8>,
    #[rustradio(out)]
    dst: WriteStream<u8>,
    #[rustradio(default)]
    out: u8,
}

impl NrziEncode {
    fn process_sync(&mut self, a: u8) -> u8 {
        if a == 0 {
            self.out ^= 1;
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;
    use crate::block::Block;
    use crate::blocks::VectorSource;

    #[test]
    fn encode() -> Result<()> {
        let (mut b, prev) = VectorSource::new(vec![0u8, 0, 0, 0, 1, 1, 1, 1]);
        b.work()?;
        let (mut b, out) = NrziDecode::new(prev);
        b.work()?;
        let (o, _) = out.read_buf()?;
        assert_eq!(o.slice(), &[1, 1, 1, 1, 0, 1, 1, 1]);
        Ok(())
    }

    #[test]
    fn decode() -> Result<()> {
        let (mut b, prev) = VectorSource::new(vec![1u8, 1, 1, 1, 0, 1, 1, 1]);
        b.work()?;
        let (mut b, out) = NrziEncode::new(prev);
        b.work()?;
        let (o, _) = out.read_buf()?;
        assert_eq!(o.slice(), &[0, 0, 0, 0, 1, 1, 1, 1]);
        Ok(())
    }

    #[test]
    fn long() -> Result<()> {
        use rand::Rng;
        let mut rng = rand::rng();
        let len = 1000;
        let data: Vec<_> = (0..len).map(|_| rng.random_range(0..=1)).collect();
        let (mut b, prev) = VectorSource::new(data.clone());
        b.work()?;
        let (mut b, prev) = NrziEncode::new(prev);
        b.work()?;
        let (mut b, out) = NrziDecode::new(prev);
        b.work()?;
        let (out, _) = out.read_buf()?;
        let out = out.slice();
        assert_eq!(out, data);
        Ok(())
    }
}
