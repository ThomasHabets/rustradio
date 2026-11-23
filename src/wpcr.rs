/*! Whole packet clock recovery block.

This is a pretty cool way to take a burst of [NRZ][nrz] floating point
samples, and turn them into bits.

Instead of a bunch of timing error detectors, symbol shapes, and loop
bandwidths, this takes the burst as a whole, and extracts the bits
according to what the whole packet looks like.

You don't even have to specify the baud rate! Though a possible
improvement could be to discard baud rates that are outside an
accepted range.

The method is this:

1. Generate a new vector marking zero crossings with 1.0, and
   everything else as 0.0.
2. Take FFT of this vector.
3. Select the "best" FFT bin, giving you both frequency and clock
   phase.
4. Extract symbols according to this frequency and clock phase.

See [Michael Ossmann's excellent presentation][video] for a better
description.

Drawbacks of this method:
* Probably less efficient.
* Probably less able to dig values out of the noise.
* Higher latency, as it needs the whole burst before it can start
  decoding.
* Uses more memory, since the whole burst needs to be in a buffer
  before decoding can start.
* Will work poorly if frequency drifts during the packet burst.

[nrz]: https://en.wikipedia.org/wiki/Non-return-to-zero
[video]: https://youtu.be/rQkBDMeODHc
 */
use log::{debug, trace, warn};

use crate::block::{Block, BlockRet};
use crate::stream::{NCReadStream, NCWriteStream, Tag, TagValue};
use crate::{Complex, Float, Result};

/// Midpointer is a block re-center a NRZ burst around 0.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Midpointer {
    #[rustradio(in)]
    src: NCReadStream<Vec<Float>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<Float>>,
}
impl Block for Midpointer {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        if self.dst.remaining() == 0 {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let (v, tags) = match self.src.pop() {
            None => return Ok(BlockRet::WaitForStream(&self.src, 1)),
            Some(burst) => burst,
        };
        let mean: Float = v.iter().sum::<Float>() / v.len() as Float;
        if mean.is_nan() {
            warn!("Midpointer got NaN");
        } else {
            let (mut a, mut b): (Vec<Float>, Vec<Float>) = v.iter().partition(|&t| *t > mean);
            if a.is_empty() || b.is_empty() {
                warn!("Midpointer got a burst without samples on both sides of mean");
                return Ok(BlockRet::Again);
            }
            a.sort_by(|a, b| a.total_cmp(b));
            b.sort_by(|a, b| a.total_cmp(b));
            let high = a[a.len() / 2];
            let low = b[b.len() / 2];
            let offset = low + (high - low) / 2.0;
            // TODO: record position of burst.
            self.dst
                .push(v.iter().map(|t| t - offset).collect::<Vec<_>>(), tags);
        }
        Ok(BlockRet::Again)
    }
}

/// Builder for Wpcr blocks.
pub struct WpcrBuilder {
    wpcr: Wpcr,
    out: NCReadStream<Vec<Float>>,
}

impl WpcrBuilder {
    /// Set sample rate. Used to tag with frequency.
    #[must_use]
    pub fn samp_rate(mut self, s: Float) -> WpcrBuilder {
        self.wpcr.set_samp_rate(Some(s));
        self
    }

    /// Build Wpcr block.
    #[must_use]
    pub fn build(self) -> (Wpcr, NCReadStream<Vec<Float>>) {
        (self.wpcr, self.out)
    }
}

/// Whole packet clock recovery block.
/// Burst tags accompany the first selected symbol at or after their input
/// sample. Tags beyond the last recovered symbol are discarded.
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct Wpcr {
    #[rustradio(in)]
    src: NCReadStream<Vec<Float>>,
    #[rustradio(out)]
    dst: NCWriteStream<Vec<Float>>,
    #[rustradio(default)]
    samp_rate: Option<Float>,
}

impl Wpcr {
    /// Create new `WpcrBuilder`.
    #[must_use]
    pub fn builder(src: NCReadStream<Vec<Float>>) -> WpcrBuilder {
        let (wpcr, out) = Wpcr::new(src);
        WpcrBuilder { wpcr, out }
    }

    /// Set sample rate. Only used for tagging purposes
    pub fn set_samp_rate(&mut self, s: Option<Float>) {
        self.samp_rate = s;
    }

    fn process_one(
        &self,
        samples: &[Float],
        input_tags: Vec<Tag>,
    ) -> Option<(Vec<Float>, Vec<Tag>)> {
        if samples.len() < 4 {
            return None;
        }

        // Unlike mossmann's version, we don't calculate the midpoint.
        // We leave it that frequency lock (assuming this is FSK) to a
        // prior block.
        let mid = 0.0;

        // Turn zero transitions into pulses.
        let sliced = samples.iter().map(|v| if *v > mid { 1.0 } else { 0.0 });
        let sliced_delayed = sliced.clone().skip(1);
        let mut d = sliced
            .zip(sliced_delayed)
            .map(|(a, b)| {
                let x = a - b;
                Complex::new(x * x, 0.0)
            })
            .collect::<Vec<_>>();

        // FFT.
        // TODO: Maybe we can pad to a power of two, to improve performance?
        let mut planner = rustfft::FftPlanner::new();
        let fft = planner.plan_fft_forward(d.len());
        fft.process(&mut d);
        d.truncate(d.len() / 2);

        // Find best match.
        let Some(bin) = find_best_bin(&d) else {
            trace!("No best bin found, giving up on burst");
            return None;
        };

        // Translate frequency and phase.
        let samples_per_symbol = bin as Float / fft.len() as Float;
        let mut clock_phase = {
            let t = 0.5 + d[bin].arg() / (std::f64::consts::PI * 2.0) as Float;
            if t > 0.5 { t } else { t + 1.0 }
        };
        debug!("WPCR: sps: {samples_per_symbol}");
        if let Some(samp_rate) = self.samp_rate {
            let frequency = samples_per_symbol * samp_rate;
            debug!("WPCR: Frequency: {frequency} Hz");
        }
        debug!("WPCR: Phase: {} rad", d[bin].arg());

        // Extract symbols.
        let mut syms =
            Vec::with_capacity((samples.len() as Float * samples_per_symbol) as usize + 10);
        let mut input_tags = input_tags.into_iter().peekable();
        let mut forwarded = Vec::new();
        for (pos, s) in samples.iter().enumerate() {
            if clock_phase >= 1.0 {
                clock_phase -= 1.0;
                while let Some(mut tag) = input_tags.next_if(|t| t.pos() <= pos) {
                    tag.set_pos(syms.len());
                    forwarded.push(tag);
                }
                syms.push(*s);
            }
            clock_phase += samples_per_symbol;
        }
        let mut tags = vec![
            Tag::new(0, "sps", TagValue::Float(samples_per_symbol)),
            Tag::new(0, "phase", TagValue::Float(clock_phase)),
        ];
        if let Some(samp_rate) = self.samp_rate {
            let frequency = samples_per_symbol * samp_rate;
            tags.push(Tag::new(0, "frequency", TagValue::Float(frequency)));
        }
        debug!("WPCR: Bits: {}", syms.len());
        tags.append(&mut forwarded);
        Some((syms, tags))
    }
}

impl Block for Wpcr {
    fn work(&mut self) -> Result<BlockRet<'_>> {
        if self.dst.remaining() == 0 {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let (x, tags) = match self.src.pop() {
            None => return Ok(BlockRet::WaitForStream(&self.src, 1)),
            Some(burst) => burst,
        };
        if let Some((packet, tags)) = self.process_one(&x, tags) {
            self.dst.push(packet, tags);
        }
        Ok(BlockRet::Again)
    }
}

fn find_best_bin(data: &[Complex]) -> Option<usize> {
    // Never select the first two buckets.
    let skip = 2;

    // Convert to magnitude.
    let mag = data.iter().map(|x| x.norm_sqr().sqrt()).collect::<Vec<_>>();

    // We want a value above 80% of max.
    let thresh = mag
        .iter()
        .take(data.len())
        .skip(skip)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))?
        * 0.8;

    // Pick the first value that's above 80% of max and not still heading upwards.
    for (n, (v, nxt)) in mag.iter().zip(mag.iter().skip(1)).enumerate().skip(skip) {
        if *v > thresh && *v > *nxt {
            return Some(n);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::new_nocopy_stream;

    #[test]
    fn maps_burst_tags_to_the_actual_selected_symbols() -> Result<()> {
        let (tx, rx) = new_nocopy_stream();
        let (mut block, out) = Wpcr::new(rx);
        // Preserve the periodic sign transitions while encoding the input
        // index in the magnitude so each selected sample can be identified.
        let samples = (0..65)
            .map(|i| {
                if (i / 4) % 2 == 0 {
                    -(i as Float + 1.0)
                } else {
                    i as Float + 1.0
                }
            })
            .collect::<Vec<_>>();
        let markers = [0, 3, 4, 17, 64];
        tx.push(
            samples,
            markers
                .map(|pos| Tag::new(pos, "input", TagValue::U64(pos as u64)))
                .to_vec(),
        );
        block.work()?;
        let (symbols, tags) = out.pop().unwrap();
        let expected = markers
            .into_iter()
            .filter_map(|i| {
                symbols
                    .iter()
                    .position(|s| s.abs() as usize > i)
                    .map(|pos| Tag::new(pos, "input", TagValue::U64(i as u64)))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            tags.into_iter()
                .filter(|t| t.key() == "input")
                .collect::<Vec<_>>(),
            expected
        );
        Ok(())
    }
    #[test]
    fn low_symbol_rate_does_not_overallocate_output() {
        let (_tx, rx) = new_nocopy_stream();
        let (block, _out) = Wpcr::new(rx);
        let samples: Vec<Float> = (0..8193)
            .map(|n| if (n / 128) % 2 == 0 { -1.0 } else { 1.0 })
            .collect();
        let (symbols, _) = block
            .process_one(&samples, Vec::new())
            .expect("periodic signal");
        assert_eq!(symbols.len(), 64);
        assert!(
            symbols.capacity() <= samples.len(),
            "capacity {}",
            symbols.capacity()
        );
    }

    #[test]
    fn frequency_uses_transition_fft_length() {
        let (_tx, rx) = new_nocopy_stream();
        let (block, _out) = Wpcr::builder(rx).samp_rate(4800.0).build();
        // 64 transition samples give an exact FFT bin at 1/4 cycles/sample.
        let samples: Vec<Float> = (0..65)
            .map(|n| if (n / 4) % 2 == 0 { -1.0 } else { 1.0 })
            .collect();
        let (_, tags) = block
            .process_one(&samples, Vec::new())
            .expect("periodic signal");
        let frequency = tags.iter().find(|tag| tag.key() == "frequency").unwrap();
        assert_eq!(frequency.val(), &TagValue::Float(1200.0));
    }
}

#[cfg(test)]
mod midpointer_tests {
    use super::*;
    #[test]
    fn forwards_tags_on_recentered_samples() -> Result<()> {
        let (writer, input) = crate::stream::new_nocopy_stream();
        let (mut block, output) = Midpointer::new(input);
        let tags = vec![Tag::new(1, "marker", TagValue::Bool(true))];
        writer.push(vec![1.0, 3.0], tags.clone());
        block.work()?;
        assert_eq!(output.pop(), Some((vec![-1.0, 1.0], tags)));
        Ok(())
    }
}
