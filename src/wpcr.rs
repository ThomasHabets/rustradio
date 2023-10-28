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
use log::{debug, warn};

use crate::block::{Block, BlockRet};
use crate::stream::{new_streamp, Streamp, Tag, TagValue};
use crate::{Complex, Error, Float};

/// Midpointer is a block re-center a NRZ burst around 0.
pub struct Midpointer {
    src: Streamp<Vec<Float>>,
    dst: Streamp<Vec<Float>>,
}
impl Midpointer {
    /// Create new midpointer.
    pub fn new(src: Streamp<Vec<Float>>) -> Self {
        Self {
            src,
            dst: new_streamp(),
        }
    }
    /// Get output stream.
    pub fn out(&self) -> Streamp<Vec<Float>> {
        self.dst.clone()
    }
}
impl Block for Midpointer {
    fn block_name(&self) -> &'static str {
        "Midpointer"
    }

    fn work(&mut self) -> Result<BlockRet, Error> {
        let mut i = self.src.lock()?;
        if i.available() == 0 {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.lock()?;
        for v in i.iter() {
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
                o.push(v.iter().map(|t| t - offset).collect::<Vec<_>>());
            }
        }
        i.clear();
        Ok(BlockRet::Ok)
    }
}

/// Builder for Wpcr blocks.
pub struct WpcrBuilder {
    wpcr: Wpcr,
}

impl WpcrBuilder {
    /// Create new WpcrBuilder
    pub fn new(src: Streamp<Vec<Float>>) -> Self {
        Self {
            wpcr: Wpcr::new(src),
        }
    }

    /// Set sample rate. Used to tag with frequency.
    pub fn samp_rate(mut self, s: Float) -> WpcrBuilder {
        self.wpcr.set_samp_rate(Some(s));
        self
    }

    /// Build Wpcr block.
    pub fn build(self) -> Wpcr {
        self.wpcr
    }
}

/// Whole packet clock recovery block.
pub struct Wpcr {
    src: Streamp<Vec<Float>>,
    dst: Streamp<Vec<u8>>,
    samp_rate: Option<Float>,
}

impl Wpcr {
    /// Create new WPCR block.
    pub fn new(src: Streamp<Vec<Float>>) -> Self {
        Self {
            src,
            dst: new_streamp(),
            samp_rate: None,
        }
    }

    /// Set sample rate. Only used for tagging purposes
    pub fn set_samp_rate(&mut self, s: Option<Float>) {
        self.samp_rate = s;
    }

    /// Return the output stream.
    pub fn out(&self) -> Streamp<Vec<u8>> {
        self.dst.clone()
    }

    fn process_one(&self, samples: &[Float]) -> Option<(Vec<u8>, Vec<Tag>)> {
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
                eprintln!("No best bin");
                return None;
            }
        };

        // Translate frequency and phase.
        let samples_per_symbol = bin as Float / samples.len() as Float;
        let mut clock_phase = {
            let t = 0.5 + d[bin].arg() / (std::f64::consts::PI * 2.0) as Float;
            if t > 0.5 {
                t
            } else {
                t + 1.0
            }
        };
        debug!("WPCR: sps: {}", samples_per_symbol);
        if let Some(samp_rate) = self.samp_rate {
            let frequency = samples_per_symbol * samp_rate;
            debug!("WPCR: Frequency: {} Hz", frequency);
        }
        debug!("WPCR: Phase: {} rad", d[bin].arg());

        // Extract symbols.
        let mut syms =
            Vec::with_capacity((samples.len() as Float / samples_per_symbol) as usize + 10);
        for s in samples {
            if clock_phase >= 1.0 {
                clock_phase -= 1.0;
                syms.push(if *s > 0.0 { 1 } else { 0 });
            }
            clock_phase += samples_per_symbol;
        }
        let mut tags = vec![
            Tag::new(0, "sps".to_string(), TagValue::Float(samples_per_symbol)),
            Tag::new(0, "phase".to_string(), TagValue::Float(clock_phase)),
        ];
        if let Some(samp_rate) = self.samp_rate {
            let frequency = samples_per_symbol * samp_rate;
            tags.push(Tag::new(
                0,
                "frequency".to_string(),
                TagValue::Float(frequency),
            ));
        }
        Some((syms, tags))
    }
}

impl Block for Wpcr {
    fn block_name(&self) -> &'static str {
        "WPCR"
    }

    fn work(&mut self) -> Result<BlockRet, Error> {
        let c = self.src.clone();
        let mut i = c.lock().unwrap();
        if i.is_empty() {
            return Ok(BlockRet::Noop);
        }
        let mut o = self.dst.lock()?;
        i.iter().for_each(|x| {
            if let Some((packet, tags)) = self.process_one(x) {
                o.push_tags(packet, &tags);
            }
        });
        i.clear();
        Ok(BlockRet::Ok)
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
