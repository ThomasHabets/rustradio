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
    fn work(&mut self) -> Result<BlockRet> {
        let v = match self.src.pop() {
            None => return Ok(BlockRet::WaitForStream(&self.src, 1)),
            Some((x, _tags)) => x,
        };
        let mean: Float = v.iter().sum::<Float>() / v.len() as Float;
        if mean.is_nan() {
            warn!("Midpointer got NaN");
        } else {
            let (mut a, mut b): (Vec<Float>, Vec<Float>) = v.iter().partition(|&t| *t > mean);
            a.sort_by(|a, b| a.partial_cmp(b).unwrap());
            b.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let high = a[a.len() / 2];
            let low = b[b.len() / 2];
            let offset = low + (high - low) / 2.0;
            // TODO: record position of burst.
            self.dst
                .push(v.iter().map(|t| t - offset).collect::<Vec<_>>(), &[]);
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
    pub fn samp_rate(mut self, s: Float) -> WpcrBuilder {
        self.wpcr.set_samp_rate(Some(s));
        self
    }

    /// Build Wpcr block.
    pub fn build(self) -> (Wpcr, NCReadStream<Vec<Float>>) {
        (self.wpcr, self.out)
    }
}

/// Whole packet clock recovery block.
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
    /// Create new WpcrBuilder.
    #[must_use]
    pub fn builder(src: NCReadStream<Vec<Float>>) -> WpcrBuilder {
        let (wpcr, out) = Wpcr::new(src);
        WpcrBuilder { wpcr, out }
    }

    /// Set sample rate. Only used for tagging purposes
    pub fn set_samp_rate(&mut self, s: Option<Float>) {
        self.samp_rate = s;
    }

    fn process_one(&self, samples: &[Float]) -> Option<(Vec<Float>, Vec<Tag>)> {
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
        let bin = match find_best_bin(&d) {
            Some(bin) => bin,
            None => {
                trace!("No best bin found, giving up on burst");
                return None;
            }
        };

        // Translate frequency and phase.
        let samples_per_symbol = bin as Float / samples.len() as Float;
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
            Vec::with_capacity((samples.len() as Float / samples_per_symbol) as usize + 10);
        for s in samples {
            if clock_phase >= 1.0 {
                clock_phase -= 1.0;
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
        Some((syms, tags))
    }
}

impl Block for Wpcr {
    fn work(&mut self) -> Result<BlockRet> {
        // TODO: handle tags.
        let x = match self.src.pop() {
            None => return Ok(BlockRet::WaitForStream(&self.src, 1)),
            Some((x, _tags)) => x,
        };
        if let Some((packet, tags)) = self.process_one(&x) {
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
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap()
        * 0.8;

    // Pick the first value that's above 80% of max and not still heading upwards.
    for (n, (v, nxt)) in mag.iter().zip(mag.iter().skip(1)).enumerate().skip(skip) {
        if *v > thresh && *v > *nxt {
            return Some(n);
        }
    }
    None
}
