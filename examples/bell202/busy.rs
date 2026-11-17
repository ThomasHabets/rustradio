//! Receive-power carrier detection and its optional graph branch.
use anyhow::{Result, ensure};
use rustradio::Sample;
use rustradio::blocks::{Map, Tee};
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::stream::ReadStream;

pub(super) type Receive<T> = (ReadStream<T>, Option<ReadStream<u8>>);

/// A 10 ms power average with 3 dB of clearance hysteresis. Power is relative
/// to unit samples, not calibrated receiver dBm. The state starts clear.
struct Detector {
    alpha: f64,
    average: f64,
    assert_power: f64,
    clear_power: f64,
    busy: bool,
}

impl Detector {
    fn new(rate: f64, threshold_db: f64) -> Result<Self> {
        ensure!(
            rate.is_finite() && rate > 0.0,
            "busy detector rate must be finite and positive"
        );
        let assert_power = 10f64.powf(threshold_db / 10.0);
        let clear_power = 10f64.powf((threshold_db - 3.0) / 10.0);
        ensure!(
            threshold_db.is_finite()
                && assert_power.is_finite()
                && assert_power > 0.0
                && clear_power.is_finite()
                && clear_power > 0.0,
            "busy threshold must be finite and yield positive, finite power thresholds"
        );
        Ok(Self {
            alpha: -(-1.0 / (rate * 0.010)).exp_m1(),
            average: 0.0,
            assert_power,
            clear_power,
            busy: false,
        })
    }

    fn process(&mut self, power: f64) -> u8 {
        // Invalid hardware samples must not poison the filter indefinitely.
        let power = if power.is_finite() && power >= 0.0 {
            power
        } else {
            0.0
        };
        self.average += self.alpha * (power - self.average);
        if self.average >= self.assert_power {
            self.busy = true;
        } else if self.average <= self.clear_power {
            self.busy = false;
        }
        u8::from(self.busy)
    }
}

/// Validate the CLI threshold before opening hardware or accepting clients.
pub(super) fn parse_threshold(value: &str) -> std::result::Result<f64, String> {
    let threshold = value.parse::<f64>().map_err(|e| e.to_string())?;
    Detector::new(50_000.0, threshold).map_err(|e| e.to_string())?;
    Ok(threshold)
}

/// Leave the receive path untouched when disabled. Otherwise fork the samples
/// and report one state per sample, so packet absence never stalls detection.
pub(super) fn tap<T: Sample>(
    graph: &mut MTGraph,
    input: ReadStream<T>,
    rate: f64,
    threshold: Option<f64>,
    power: fn(T) -> f64,
) -> Result<Receive<T>> {
    let Some(threshold) = threshold else {
        return Ok((input, None));
    };
    let mut detector = Detector::new(rate, threshold)?;
    let mut state_samples = 0u64;
    let (tee, data, level) = Tee::new(input)?;
    graph.add(Box::new(tee));
    let (map, busy) = Map::keep_tags(level, "ChannelBusy", move |value| {
        let was_busy = detector.busy;
        let state = detector.process(power(value));
        if detector.busy != was_busy {
            let (current, previous) = if detector.busy {
                ("busy", "idle")
            } else {
                ("idle", "busy")
            };
            // Use sample time so buffering and scheduler delays do not distort
            // the length of the received busy or idle period.
            println!(
                "Channel {current} ({previous} for {:.3}s)",
                state_samples as f64 / rate
            );
            state_samples = 0;
        }
        state_samples += 1;
        state
    })?;
    graph.add(Box::new(map));
    Ok((data, Some(busy)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hysteresis_and_invalid_samples_recover() -> Result<()> {
        let mut detector = Detector::new(1000.0, -20.0)?;
        assert_eq!(detector.process(0.0), 0);
        assert_eq!(detector.process(1.0), 1);
        for _ in 0..1000 {
            assert_eq!(detector.process(0.0075), 1);
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            detector.process(value);
            assert!(detector.average.is_finite());
        }
        for _ in 0..1000 {
            detector.process(0.0);
        }
        assert_eq!(detector.process(0.0), 0);
        assert_eq!(detector.process(1.0), 1);
        Ok(())
    }

    #[test]
    fn smoothing_tracks_seconds_instead_of_samples() -> Result<()> {
        for rate in [8000.0, 44100.0, 48000.0, 50000.0] {
            let mut detector = Detector::new(rate, 0.0)?;
            for _ in 0..(rate / 10.0) as usize {
                detector.process(1.0);
            }
            assert!((detector.average - (1.0 - (-10f64).exp())).abs() < 1e-12);
        }
        Ok(())
    }

    #[test]
    fn signal_without_packets_emits_clear_busy_clear() -> Result<()> {
        use rustradio::Float;
        use rustradio::blocks::{KissEncode, NullSink, PduToStream, VectorSink, VectorSource};
        use rustradio::stream::new_nocopy_stream;
        let rate = 48000;
        let mut samples = vec![0.0; rate / 10];
        samples.extend((0..rate / 10).map(|i| {
            (0.5 * (i as f64 * 1200.0 * std::f64::consts::TAU / rate as f64).sin()) as Float
        }));
        samples.extend(vec![0.0; rate / 5]);
        let mut graph = MTGraph::new();
        let (source, input) = VectorSource::new(samples)?;
        graph.add(Box::new(source));
        let (data, busy) = tap(&mut graph, input, rate as f64, Some(-30.0), |sample| {
            f64::from(sample).powi(2)
        })?;
        graph.add(Box::new(NullSink::new(data)));
        let (writer, packets) = new_nocopy_stream();
        drop(writer);
        let (encoder, frames) = KissEncode::new(packets);
        graph.add(Box::new(encoder.with_busy(busy.unwrap())));
        let (flatten, wire) = PduToStream::new(frames)?;
        graph.add(Box::new(flatten));
        let sink = VectorSink::new(wire, 100);
        let hook = sink.hook();
        graph.add(Box::new(sink));
        graph.run()?;
        assert_eq!(
            hook.data().samples(),
            &[
                0xc0, 6, 0x8e, 0, 0xc0, 0xc0, 6, 0x8e, 1, 0xc0, 0xc0, 6, 0x8e, 0, 0xc0
            ]
        );
        Ok(())
    }

    #[test]
    fn disabled_tap_preserves_the_input() -> Result<()> {
        let (writer, input) = rustradio::stream::new_stream()?;
        let mut window = writer.write_buf()?;
        window.slice()[..2].copy_from_slice(&[0.25f32, -0.5]);
        window.produce(2, &[]);
        let mut graph = MTGraph::new();
        let (data, busy) = tap(&mut graph, input, 48000.0, None, |sample| {
            f64::from(sample).powi(2)
        })?;
        assert!(busy.is_none());
        assert_eq!(data.read_buf()?.0.slice(), &[0.25, -0.5]);
        Ok(())
    }

    #[test]
    fn validates_configuration() {
        for value in ["NaN", "inf", "-inf", "10000", "-10000", "garbage"] {
            assert!(parse_threshold(value).is_err(), "{value}");
        }
        assert_eq!(parse_threshold("-30").unwrap(), -30.0);
        assert!(Detector::new(0.0, -30.0).is_err());
        assert!(Detector::new(f64::NAN, -30.0).is_err());
    }
}
