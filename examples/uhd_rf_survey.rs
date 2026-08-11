//! Sweep a frequency range with timed UHD retunes and record averaged PSD.

use std::collections::BTreeMap;
use std::io::{BufWriter, Write};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use clap::Parser;
use log::{debug, info, warn};
use rustfft::FftPlanner;

use rustradio::block::{Block, BlockRet};
use rustradio::blocks::{UhdDevice, UhdSource};
use rustradio::graph::{CancellationToken, GraphRunner};
use rustradio::mtgraph::MTGraph;
use rustradio::parse_frequency;
use rustradio::stream::{ReadStream, TagValue};
use rustradio::window::WindowType;
use rustradio::{Complex, Error, blockchain};

const CHANNEL: usize = 0;
const NANOS_PER_SECOND: u64 = 1_000_000_000;
const TIME_TAG: &str = "UhdSource::time_ns";
const ERROR_KIND_TAG: &str = "UhdSource::error_kind";
const OUT_OF_SEQUENCE_TAG: &str = "UhdSource::out_of_sequence";
const LO_LOCKED_SENSOR: &str = "lo_locked";
const SENSOR_POLL_INTERVAL: Duration = Duration::from_millis(1);
const SURVEY_WORK_SAMPLES: usize = 65_536;
const OUTPUT_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Sweep a USRP across a wide RF range and write averaged PSD rows"
)]
struct Opt {
    /// UHD device address string.
    #[arg(long, default_value = "type=b200")]
    device: String,

    /// Output text file. Existing files are not overwritten.
    #[arg(short, long)]
    output: PathBuf,

    /// Write one whole-run average/max row per frequency instead of history.
    ///
    /// Summary rows are written when a finite survey ends or after Ctrl-C.
    /// Abrupt process termination can leave the output incomplete.
    #[arg(long)]
    summarize: bool,

    /// Lower measured-frequency edge in Hz.
    #[arg(long, value_parser = parse_frequency)]
    start_frequency: f64,

    /// Upper measured-frequency edge in Hz.
    #[arg(long, value_parser = parse_frequency)]
    stop_frequency: f64,

    /// Requested receive sample rate in samples per second.
    ///
    /// The step size becomes:
    /// `(stop-start)/ceil((stop-start) / (0.8 * sample_rate))`
    #[arg(long, value_parser = parse_frequency, default_value = "8M")]
    sample_rate: f64,

    /// Maximum spacing between adjacent tuner centers.
    ///
    /// By default this is 80% of the actual device sample rate.
    #[arg(long, value_parser = parse_frequency)]
    step: Option<f64>,

    /// Offset between the requested center and the hardware RF LO.
    ///
    /// By default this is halfway between the retained-band edge and Nyquist,
    /// which moves the receiver's DC spur outside the frequencies being saved.
    /// Set this to zero to keep the RF LO at the requested center.
    #[arg(long, value_parser = parse_frequency, allow_hyphen_values = true)]
    lo_offset: Option<f64>,

    /// Keep the configured LO-offset sign instead of alternating it by sweep.
    #[arg(long)]
    fixed_lo_offset: bool,

    /// Time spent at each tuner center.
    #[arg(long, value_parser = humantime::parse_duration, default_value = "200ms")]
    dwell: Duration,

    /// Samples this long after each retune are discarded.
    ///
    /// B200 low-IF timed retunes can remain transient for more than 50 ms.
    #[arg(long, value_parser = humantime::parse_duration, default_value = "70ms")]
    settle: Duration,

    /// How far ahead each timed tune is submitted to UHD.
    #[arg(long, value_parser = humantime::parse_duration, default_value = "50ms")]
    command_lead: Duration,

    /// FFT length.
    #[arg(long, default_value_t = 4096)]
    fft_size: usize,

    /// Number of complete sweeps. Omit to sweep until Ctrl-C.
    #[arg(long)]
    sweeps: Option<NonZeroU64>,

    /// Receive gain in dB.
    #[arg(long, default_value_t = 30.0)]
    gain: f64,

    /// Receive antenna. By default the device's current antenna is kept.
    #[arg(long)]
    antenna: Option<String>,

    /// Clock source for motherboard 0, such as `gpsdo`.
    #[arg(long)]
    clock_source: Option<String>,

    /// Time source for motherboard 0, such as `gpsdo`.
    #[arg(long)]
    time_source: Option<String>,

    /// Logging verbosity.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

/// One tuner center and the absolute-frequency interval assigned to it.
#[derive(Clone, Debug, PartialEq)]
struct ScanBand {
    center: f64,
    low: f64,
    high: f64,
    includes_high_edge: bool,
}

/// Immutable frequency and repetition plan shared by the tuner and sink.
#[derive(Debug)]
struct ScanPlan {
    bands: Vec<ScanBand>,
    total_dwells: Option<u64>,
}

impl ScanPlan {
    /// Divide a requested coverage range among tuner centers without gaps.
    fn new(
        start: f64,
        stop: f64,
        step: f64,
        sample_rate: f64,
        sweeps: Option<NonZeroU64>,
    ) -> Result<Self> {
        ensure!(
            start.is_finite() && start > 0.0,
            "start frequency must be positive and finite"
        );
        ensure!(
            stop.is_finite() && stop > start,
            "stop frequency must be greater than start frequency"
        );
        ensure!(
            step.is_finite() && step > 0.0,
            "frequency step must be positive and finite"
        );
        ensure!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample rate must be positive and finite"
        );
        ensure!(
            step <= sample_rate,
            "frequency step {step} exceeds actual sample rate {sample_rate}"
        );

        let width = stop - start;
        ensure!(width.is_finite(), "frequency range is too wide");
        let band_count_f64 = (width / step).ceil().max(1.0);
        ensure!(
            band_count_f64 <= usize::MAX as f64,
            "frequency range requires too many tuner centers"
        );
        let band_count = band_count_f64 as usize;
        let band_width = width / band_count as f64;

        let mut bands = Vec::with_capacity(band_count);
        for index in 0..band_count {
            let low = start + index as f64 * band_width;
            let includes_high_edge = index + 1 == band_count;
            let high = if includes_high_edge {
                stop
            } else {
                start + (index + 1) as f64 * band_width
            };
            bands.push(ScanBand {
                center: (low + high) / 2.0,
                low,
                high,
                includes_high_edge,
            });
        }
        info!("Bands:");
        for (n, band) in bands.iter().enumerate() {
            info!("{n}: {band:?}");
        }

        let total_dwells = sweeps
            .map(|count| {
                count
                    .get()
                    .checked_mul(u64::try_from(bands.len())?)
                    .context("number of survey dwells overflows u64")
            })
            .transpose()?;
        Ok(Self {
            bands,
            total_dwells,
        })
    }

    /// Return the band used by a monotonically increasing dwell index.
    fn band(&self, dwell_index: u64) -> &ScanBand {
        &self.bands[dwell_index as usize % self.bands.len()]
    }

    /// Return whether this index lies after a configured finite survey.
    fn finished(&self, dwell_index: u64) -> bool {
        self.total_dwells.is_some_and(|total| dwell_index >= total)
    }
}

/// Accumulate windowed FFT periodograms and average them in linear units.
struct SpectrumAverager {
    sample_rate: f64,
    fft_size: usize,
    window: Vec<f32>,
    window_energy: f64,
    fft: Arc<dyn rustfft::Fft<f32>>,
    frame: Vec<Complex>,
    work: Vec<Complex>,
    power_sum: Vec<f64>,
    frames: u64,
}

impl SpectrumAverager {
    /// Build a Blackman-Harris periodogram averager.
    fn new(sample_rate: f64, fft_size: usize) -> Result<Self> {
        ensure!(fft_size > 0, "FFT size must be greater than zero");
        let window = WindowType::BlackmanHarris.make_window(fft_size).0;
        let window_energy = window
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>();
        ensure!(window_energy > 0.0, "FFT window has zero energy");
        let fft = FftPlanner::new().plan_fft_forward(fft_size);
        Ok(Self {
            sample_rate,
            fft_size,
            window,
            window_energy,
            fft,
            frame: Vec::with_capacity(fft_size),
            work: vec![Complex::default(); fft_size],
            power_sum: vec![0.0; fft_size],
            frames: 0,
        })
    }

    /// Add one time-domain sample and process a complete FFT frame.
    fn push(&mut self, sample: Complex) {
        self.frame.push(sample);
        if self.frame.len() != self.fft_size {
            return;
        }
        if self
            .frame
            .iter()
            .all(|sample| *sample == Complex::default())
        {
            self.frame.clear();
            return;
        }
        for ((dst, src), window) in self.work.iter_mut().zip(&self.frame).zip(&self.window) {
            *dst = *src * *window;
        }
        self.fft.process(&mut self.work);
        for (sum, bin) in self.power_sum.iter_mut().zip(&self.work) {
            *sum += f64::from(bin.norm_sqr());
        }
        self.frames += 1;
        self.frame.clear();
    }

    /// Finish the average, returning bins in rustfft's unshifted order.
    fn finish(&mut self) -> Option<Vec<f64>> {
        if self.frames == 0 {
            self.reset();
            return None;
        }
        let scale = self.sample_rate * self.window_energy * self.frames as f64;
        let result = self.power_sum.iter().map(|power| power / scale).collect();
        self.reset();
        Some(result)
    }

    /// Discard all complete and partial frames.
    fn reset(&mut self) {
        self.frame.clear();
        self.power_sum.fill(0.0);
        self.frames = 0;
    }
}

/// One completed dwell ready to be written.
struct Measurement {
    dwell_index: u64,
    timestamp_ns: i64,
    psd: Vec<f64>,
}

/// Compensated linear-power statistics for one LO-offset path.
#[derive(Clone, Copy, Default)]
struct LinearAccumulator {
    count: u64,
    total: f64,
    correction: f64,
    maximum: f64,
}

impl LinearAccumulator {
    /// Add one linear-power observation using Kahan summation.
    fn add(&mut self, value: f64) {
        let adjusted = value - self.correction;
        let updated = self.total + adjusted;
        self.correction = (updated - self.total) - adjusted;
        self.total = updated;
        self.maximum = self.maximum.max(value);
        self.count += 1;
    }

    /// Return the linear arithmetic mean for a nonempty path.
    fn mean(&self) -> f64 {
        self.total / self.count as f64
    }
}

/// Whole-run statistics for one absolute-frequency bin.
struct SummaryBin {
    frequency: f64,
    // Negative, zero, and positive LO-offset paths, respectively.
    paths: [LinearAccumulator; 3],
}

impl SummaryBin {
    /// Return the number of observations across all LO paths.
    fn count(&self) -> u64 {
        self.paths.iter().map(|path| path.count).sum()
    }

    /// Return whether both alternating nonzero LO paths have data.
    fn uses_image_rejection(&self) -> bool {
        self.paths[0].count > 0 && self.paths[2].count > 0
    }

    /// Return the pooled or two-path image-rejected linear mean.
    fn mean(&self) -> f64 {
        if self.uses_image_rejection() {
            return self.paths[0].mean().min(self.paths[2].mean());
        }
        compensated_sum(
            self.paths
                .iter()
                .filter(|path| path.count > 0)
                .map(|path| path.total),
        ) / self.count() as f64
    }

    /// Return the pooled or two-path image-rejected linear maximum.
    fn maximum(&self) -> f64 {
        if self.uses_image_rejection() {
            return self.paths[0].maximum.min(self.paths[2].maximum);
        }
        self.paths
            .iter()
            .filter(|path| path.count > 0)
            .map(|path| path.maximum)
            .fold(0.0, f64::max)
    }
}

/// Compact per-frequency accumulation for an entire survey run.
struct RunSummary {
    band_bins: Vec<Vec<(usize, usize)>>,
    bins: Vec<SummaryBin>,
}

impl RunSummary {
    /// Precompute the sorted output frequencies and their FFT-bin mappings.
    fn new(plan: &ScanPlan, sample_rate: f64, fft_size: usize) -> Self {
        let mut bins = Vec::new();
        let mut band_bins = Vec::with_capacity(plan.bands.len());
        for band in &plan.bands {
            let mut mappings = Vec::new();
            for fft_bin in shifted_indices(fft_size) {
                let frequency = band.center + bin_offset(fft_bin, fft_size, sample_rate);
                let in_band = frequency >= band.low
                    && (frequency < band.high || band.includes_high_edge && frequency <= band.high);
                if !in_band {
                    continue;
                }
                let summary_bin = bins.len();
                bins.push(SummaryBin {
                    frequency,
                    paths: [LinearAccumulator::default(); 3],
                });
                mappings.push((fft_bin, summary_bin));
            }
            band_bins.push(mappings);
        }
        Self { band_bins, bins }
    }

    /// Add one completed dwell to its LO path.
    fn add(&mut self, measurement: &Measurement, lo_offset: f64) {
        let band = measurement.dwell_index as usize % self.band_bins.len();
        let path = if lo_offset < 0.0 {
            0
        } else if lo_offset > 0.0 {
            2
        } else {
            1
        };
        for &(fft_bin, summary_bin) in &self.band_bins[band] {
            let power = measurement.psd[fft_bin];
            if power.is_finite() && power > 0.0 {
                self.bins[summary_bin].paths[path].add(power);
            }
        }
    }

    /// Write the same compact spectrum columns as the averaging script.
    fn write_rows(&self, output: &mut dyn Write) -> std::io::Result<()> {
        for bin in self.bins.iter().filter(|bin| bin.count() > 0) {
            writeln!(
                output,
                "{:.6} {:.9} {:.9} {}",
                bin.frequency,
                linear_to_db(bin.mean()),
                linear_to_db(bin.maximum()),
                bin.count(),
            )?;
        }
        Ok(())
    }
}

/// Sum a few values with the same compensation used for per-path totals.
fn compensated_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut total = 0.0;
    let mut correction = 0.0;
    for value in values {
        let adjusted = value - correction;
        let updated = total + adjusted;
        correction = (updated - total) - adjusted;
        total = updated;
    }
    total
}

/// Convert linear power to decibels, preserving exact zero as negative infinity.
fn linear_to_db(power: f64) -> f64 {
    if power == 0.0 {
        f64::NEG_INFINITY
    } else {
        10.0 * power.log10()
    }
}

/// Status sent from the timed-retune controller to the sample sink.
enum ControllerEvent {
    LoLocked { dwell_index: u64, at: uhd::TimeSpec },
    Fatal(String),
}

/// Timing and FFT settings shared by the processor and sink.
#[derive(Clone, Copy)]
struct SurveyConfig {
    start_time_ns: i64,
    dwell_ns: i64,
    settle_ns: i64,
    command_lead_ns: i64,
    sample_rate: f64,
    fft_size: usize,
}

/// Output and LO-path settings used by the survey sink.
#[derive(Clone, Copy)]
struct SurveyOutputConfig {
    lo_offset: f64,
    alternate_lo_offset: bool,
    summarize: bool,
}

/// Parameters used by the timed-retune controller.
#[derive(Clone, Copy)]
struct RetuneConfig {
    lo_offset: f64,
    alternate_lo_offset: bool,
    start_time_ns: i64,
    dwell_ns: i64,
    lead_ns: i64,
}

/// Stateful association of timestamped samples with scheduled tuner dwells.
struct SurveyProcessor {
    plan: Arc<ScanPlan>,
    start_time_ns: i64,
    dwell_ns: i64,
    settle_ns: i64,
    command_lead_ns: i64,
    active_dwell: Option<u64>,
    active_invalid: bool,
    lo_locked_at: BTreeMap<u64, uhd::TimeSpec>,
    averager: SpectrumAverager,
}

impl SurveyProcessor {
    /// Construct the sample-to-dwell processor.
    fn new(plan: Arc<ScanPlan>, config: SurveyConfig) -> Result<Self> {
        Ok(Self {
            plan,
            start_time_ns: config.start_time_ns,
            dwell_ns: config.dwell_ns,
            settle_ns: config.settle_ns,
            command_lead_ns: config.command_lead_ns,
            active_dwell: None,
            active_invalid: false,
            lo_locked_at: BTreeMap::new(),
            averager: SpectrumAverager::new(config.sample_rate, config.fft_size)?,
        })
    }

    /// Record when UHD first reported that a scheduled retune was locked.
    fn record_lo_lock(&mut self, dwell_index: u64, at: uhd::TimeSpec) -> Result<()> {
        let start = dwell_start_ns(self.start_time_ns, self.dwell_ns, dwell_index)?;
        let end = start
            .checked_add(self.dwell_ns)
            .context("survey dwell end time overflow")?;
        let time_ns = at.into_nanos();
        ensure!(
            time_ns >= start && time_ns < end,
            "LO lock timestamp {time_ns} lies outside survey dwell {dwell_index} ({start}..{end})"
        );
        if self.active_dwell.is_some_and(|active| dwell_index < active) {
            warn!("Ignoring late LO lock report for survey dwell {dwell_index}");
            return Ok(());
        }
        self.lo_locked_at.insert(dwell_index, at);
        Ok(())
    }

    /// Process one timestamped sample and optionally complete a dwell.
    fn process_sample(
        &mut self,
        time_ns: i64,
        sample: Complex,
        invalidate: bool,
    ) -> Result<(Option<Measurement>, bool)> {
        if time_ns < self.start_time_ns {
            return Ok((None, false));
        }
        let elapsed = time_ns
            .checked_sub(self.start_time_ns)
            .context("device timestamp subtraction overflow")?;
        let dwell_index = u64::try_from(elapsed / self.dwell_ns)?;

        if self.plan.finished(dwell_index) {
            return Ok((self.finish_active()?, true));
        }

        let completed = match self.active_dwell {
            None => {
                self.begin_dwell(dwell_index);
                None
            }
            Some(active) if active == dwell_index => None,
            Some(active) if active < dwell_index => {
                let measurement = self.finish_active()?;
                self.begin_dwell(dwell_index);
                measurement
            }
            Some(active) => {
                anyhow::bail!(
                    "device time moved backwards from survey dwell {active} to {dwell_index}"
                )
            }
        };

        if invalidate {
            self.active_invalid = true;
            self.averager.reset();
        }
        let dwell_start = dwell_start_ns(self.start_time_ns, self.dwell_ns, dwell_index)?;
        let settled_at = dwell_start
            .checked_add(self.settle_ns)
            .context("measurement start time overflow")?;
        let measurement_end = dwell_start
            .checked_add(self.dwell_ns)
            .and_then(|end| end.checked_sub(self.command_lead_ns))
            .context("measurement end time overflow")?;
        let measurement_start = self
            .lo_locked_at
            .get(&dwell_index)
            .map(|locked_at| settled_at.max(locked_at.into_nanos()));
        if !self.active_invalid
            && measurement_start.is_some_and(|start| time_ns >= start)
            && time_ns < measurement_end
        {
            self.averager.push(sample);
        }
        Ok((completed, false))
    }

    /// Start accumulating a new scheduled dwell.
    fn begin_dwell(&mut self, dwell_index: u64) {
        self.active_dwell = Some(dwell_index);
        self.active_invalid = false;
        self.lo_locked_at.retain(|index, _| *index >= dwell_index);
        self.averager.reset();
    }

    /// Complete the current dwell if it contains valid complete FFT frames.
    fn finish_active(&mut self) -> Result<Option<Measurement>> {
        let Some(dwell_index) = self.active_dwell.take() else {
            return Ok(None);
        };
        if self.active_invalid {
            warn!("Discarding survey dwell {dwell_index} after an RX discontinuity");
            self.lo_locked_at.remove(&dwell_index);
            self.averager.reset();
            return Ok(None);
        }
        let Some(locked_at) = self.lo_locked_at.remove(&dwell_index) else {
            warn!("Discarding survey dwell {dwell_index} without an LO lock report");
            self.averager.reset();
            return Ok(None);
        };
        let Some(psd) = self.averager.finish() else {
            warn!("Discarding survey dwell {dwell_index} with no complete FFT frame");
            return Ok(None);
        };
        if psd.iter().all(|power| *power == 0.0) {
            warn!("Discarding all-zero survey dwell {dwell_index}");
            return Ok(None);
        }
        let start = dwell_start_ns(self.start_time_ns, self.dwell_ns, dwell_index)?;
        let measurement_start = start
            .checked_add(self.settle_ns)
            .context("measurement start time overflow")?
            .max(locked_at.into_nanos());
        let measurement_end = start
            .checked_add(self.dwell_ns)
            .and_then(|end| end.checked_sub(self.command_lead_ns))
            .context("measurement end time overflow")?;
        let timestamp_ns = measurement_start
            .checked_add((measurement_end - measurement_start) / 2)
            .context("measurement timestamp overflow")?;
        Ok(Some(Measurement {
            dwell_index,
            timestamp_ns,
            psd,
        }))
    }
}

/// Sink that writes either dwell history or a compact whole-run spectrum.
///
/// It consumes `UhdSource::time_ns` and RX error tags. All other tags are
/// deliberately ignored because this sink has no output stream.
enum SurveyOutput {
    History,
    Summary(RunSummary),
    Finalized,
}

#[derive(rustradio_macros::Block)]
struct SurveySink {
    #[rustradio(in)]
    src: ReadStream<Complex>,
    plan: Arc<ScanPlan>,
    processor: SurveyProcessor,
    sample_rate: f64,
    fft_size: usize,
    writer: Box<dyn Write + Send>,
    output: PathBuf,
    output_state: SurveyOutput,
    lo_offset: f64,
    alternate_lo_offset: bool,
    controller_events: mpsc::Receiver<ControllerEvent>,
    next_time_ns: Option<i64>,
}

impl SurveySink {
    /// Open the output and construct the survey sink.
    fn new(
        src: ReadStream<Complex>,
        output: &Path,
        plan: Arc<ScanPlan>,
        config: SurveyConfig,
        output_config: SurveyOutputConfig,
        controller_events: mpsc::Receiver<ControllerEvent>,
    ) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|error| Error::file_io(error, output))?;
        let mut writer: Box<dyn Write + Send> =
            Box::new(BufWriter::with_capacity(OUTPUT_BUFFER_BYTES, file));
        let output_state = if output_config.summarize {
            writeln!(
                writer,
                "# frequency_hz average_power_dbfs_per_hz maximum_power_dbfs_per_hz observations"
            )?;
            SurveyOutput::Summary(RunSummary::new(&plan, config.sample_rate, config.fft_size))
        } else {
            writeln!(
                writer,
                "# device_time_ns frequency_hz power_dbfs_per_hz lo_offset_hz"
            )?;
            SurveyOutput::History
        };
        Ok(Self {
            src,
            processor: SurveyProcessor::new(Arc::clone(&plan), config)?,
            plan,
            sample_rate: config.sample_rate,
            fft_size: config.fft_size,
            writer,
            output: output.to_path_buf(),
            output_state,
            lo_offset: output_config.lo_offset,
            alternate_lo_offset: output_config.alternate_lo_offset,
            controller_events,
            next_time_ns: None,
        })
    }

    /// Write a completed dwell in ascending absolute-frequency order.
    fn write_measurement(&mut self, measurement: Measurement) -> rustradio::Result<()> {
        let lo_offset = lo_offset_for_dwell(
            self.lo_offset,
            self.alternate_lo_offset,
            self.plan.bands.len(),
            measurement.dwell_index,
        );
        match &mut self.output_state {
            SurveyOutput::Summary(summary) => {
                summary.add(&measurement, lo_offset);
                return Ok(());
            }
            SurveyOutput::History => {}
            SurveyOutput::Finalized => {
                return Err(Error::msg("cannot write a finalized survey output"));
            }
        }

        let band = self.plan.band(measurement.dwell_index);
        for bin in shifted_indices(self.fft_size) {
            let frequency = band.center + bin_offset(bin, self.fft_size, self.sample_rate);
            let in_band = frequency >= band.low
                && (frequency < band.high || band.includes_high_edge && frequency <= band.high);
            if !in_band {
                continue;
            }
            let power = if measurement.psd[bin] > 0.0 {
                10.0 * measurement.psd[bin].log10()
            } else {
                f64::NEG_INFINITY
            };
            writeln!(
                self.writer,
                "{} {:.6} {:.9} {:.6}",
                measurement.timestamp_ns, frequency, power, lo_offset
            )
            .map_err(|error| Error::file_io(error, &self.output))?;
        }
        Ok(())
    }

    /// Write any deferred summary rows and flush the output exactly once.
    fn finish_output(&mut self) -> rustradio::Result<()> {
        let state = std::mem::replace(&mut self.output_state, SurveyOutput::Finalized);
        let write_result = match state {
            SurveyOutput::Summary(summary) => summary
                .write_rows(self.writer.as_mut())
                .map_err(|error| Error::file_io(error, &self.output)),
            SurveyOutput::History | SurveyOutput::Finalized => Ok(()),
        };
        let flush_result = self
            .writer
            .flush()
            .map_err(|error| Error::file_io(error, &self.output));
        write_result?;
        flush_result
    }

    /// Apply all pending lock reports or return a controller error.
    fn check_controller(&mut self) -> rustradio::Result<()> {
        loop {
            match self.controller_events.try_recv() {
                Ok(ControllerEvent::LoLocked { dwell_index, at }) => self
                    .processor
                    .record_lo_lock(dwell_index, at)
                    .map_err(|error| Error::msg(format!("invalid LO lock report: {error:#}")))?,
                Ok(ControllerEvent::Fatal(error)) => {
                    return Err(Error::msg(format!(
                        "timed retune controller failed: {error}"
                    )));
                }
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
    }
}

impl Block for SurveySink {
    fn work(&mut self) -> rustradio::Result<BlockRet<'_>> {
        self.check_controller()?;
        let (input, tags) = self.src.read_buf()?;
        if input.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.src, 1));
        }
        // Consume bounded chunks so a delayed FFT or file write cannot hold
        // the stream full long enough for the USRP transport to overflow.
        let available = input.slice();
        let samples = &available[..available.len().min(SURVEY_WORK_SAMPLES)];
        let mut tags = tags
            .into_iter()
            .filter(|tag| tag.pos() < samples.len())
            .peekable();
        let mut anchor_time = self.next_time_ns;
        let mut anchor_pos = 0usize;
        let mut invalidate = false;
        let mut done = false;

        for (position, sample) in samples.iter().copied().enumerate() {
            while tags.peek().is_some_and(|tag| tag.pos() == position) {
                let tag = tags.next().expect("peeked tag exists");
                match (tag.key(), tag.val()) {
                    (TIME_TAG, TagValue::I64(time_ns)) => {
                        anchor_time = Some(*time_ns);
                        anchor_pos = position;
                    }
                    (OUT_OF_SEQUENCE_TAG, TagValue::Bool(true)) => invalidate = true,
                    (ERROR_KIND_TAG, TagValue::String(kind))
                        if kind == "overflow" || kind == "out_of_sequence" =>
                    {
                        invalidate = true;
                    }
                    (ERROR_KIND_TAG, TagValue::String(kind)) => {
                        return Err(Error::msg(format!("fatal UHD receive error: '{kind}'")));
                    }
                    _ => {}
                }
            }
            let Some(anchor_time) = anchor_time else {
                return Err(Error::msg(
                    "UHD survey received samples without a UhdSource::time_ns tag",
                ));
            };
            let offset = position - anchor_pos;
            let offset_ns = (offset as f64 * NANOS_PER_SECOND as f64 / self.sample_rate).round();
            ensure_i64(offset_ns, "sample timestamp offset")?;
            let time_ns = anchor_time
                .checked_add(offset_ns as i64)
                .ok_or_else(|| Error::msg("sample timestamp overflow"))?;
            let (measurement, reached_end) = self
                .processor
                .process_sample(time_ns, sample, invalidate)
                .map_err(|error| {
                    Error::msg(format!("failed to process UHD survey samples: {error:#}"))
                })?;
            invalidate = false;
            if let Some(measurement) = measurement {
                self.write_measurement(measurement)?;
            }
            if reached_end {
                done = true;
                break;
            }
        }

        if let Some(anchor_time) = anchor_time {
            let offset = samples.len() - anchor_pos;
            let offset_ns = (offset as f64 * NANOS_PER_SECOND as f64 / self.sample_rate).round();
            ensure_i64(offset_ns, "next sample timestamp offset")?;
            self.next_time_ns = anchor_time.checked_add(offset_ns as i64);
        }
        let count = samples.len();
        input.consume(count);
        if done {
            self.finish_output()?;
            Ok(BlockRet::EOF)
        } else {
            Ok(BlockRet::Again)
        }
    }
}

impl Drop for SurveySink {
    fn drop(&mut self) {
        if let Err(error) = self.finish_output() {
            warn!("Failed to flush {}: {error}", self.output.display());
        }
    }
}

/// Schedule every tuner center at its exact device timestamp.
fn run_retune_controller(
    device: UhdDevice,
    plan: Arc<ScanPlan>,
    config: RetuneConfig,
    events: mpsc::Sender<ControllerEvent>,
    cancel: CancellationToken,
) -> Result<()> {
    let RetuneConfig {
        lo_offset,
        alternate_lo_offset,
        start_time_ns,
        dwell_ns,
        lead_ns,
    } = config;
    let mut dwell_index = 0u64;
    while !plan.finished(dwell_index) && !cancel.is_canceled() {
        let command_time = dwell_start_ns(start_time_ns, dwell_ns, dwell_index)?;
        loop {
            if cancel.is_canceled() {
                return Ok(());
            }
            let now_ns = device
                .configure(|usrp| usrp.get_current_time(CHANNEL))?
                .into_nanos();
            ensure!(
                now_ns < command_time,
                "missed timed-retune deadline for dwell {dwell_index} by {} ns",
                now_ns - command_time
            );
            let wait_ns = command_time - now_ns - lead_ns;
            if wait_ns <= 0 {
                break;
            }
            std::thread::sleep(Duration::from_nanos(u64::try_from(
                wait_ns.min(10_000_000),
            )?));
        }

        let band = plan.band(dwell_index);
        let dwell_lo_offset = lo_offset_for_dwell(
            lo_offset,
            alternate_lo_offset,
            plan.bands.len(),
            dwell_index,
        );
        let tune = device.configure(|usrp| {
            usrp.set_command_time(uhd::TimeSpec::from_nanos(command_time), CHANNEL)?;
            let tune = usrp.set_rx_frequency(
                &uhd::TuneRequest::with_frequency_lo(band.center, dwell_lo_offset),
                CHANNEL,
            );
            let clear = usrp.clear_command_time(CHANNEL);
            match (tune, clear) {
                (Ok(tune), Ok(())) => Ok(tune),
                (Err(error), _) | (Ok(_), Err(error)) => Err(error),
            }
        })?;
        let actual_rf_lo = tune.actual_rf_freq();
        let rf_lo_in_saved_band = actual_rf_lo >= band.low
            && (actual_rf_lo < band.high || band.includes_high_edge && actual_rf_lo <= band.high);
        if dwell_lo_offset != 0.0 {
            ensure!(
                !rf_lo_in_saved_band,
                "actual RF LO {actual_rf_lo} Hz lies inside saved interval {}..{} Hz for dwell {dwell_index}",
                band.low,
                band.high
            );
        }
        debug!(
            "Scheduled dwell {dwell_index} at {command_time} ns, center {:.3} MHz, LO offset {:+.3} MHz: {tune:?}",
            band.center / 1e6,
            dwell_lo_offset / 1e6,
        );
        wait_for_lo_lock(
            &device,
            dwell_index,
            command_time,
            dwell_ns,
            &events,
            &cancel,
        )?;
        dwell_index = dwell_index
            .checked_add(1)
            .context("survey dwell index overflow")?;
    }
    Ok(())
}

/// Mirror the low-IF offset on alternate sweeps to average receiver response.
fn lo_offset_for_dwell(
    lo_offset: f64,
    alternate: bool,
    bands_per_sweep: usize,
    dwell_index: u64,
) -> f64 {
    let sweep_index = dwell_index / bands_per_sweep as u64;
    if alternate && sweep_index % 2 == 1 {
        -lo_offset
    } else {
        lo_offset
    }
}

/// Poll UHD until the scheduled retune reports an RX LO lock.
fn wait_for_lo_lock(
    device: &UhdDevice,
    dwell_index: u64,
    command_time_ns: i64,
    dwell_ns: i64,
    events: &mpsc::Sender<ControllerEvent>,
    cancel: &CancellationToken,
) -> Result<()> {
    let deadline_ns = command_time_ns
        .checked_add(dwell_ns)
        .context("LO lock deadline overflow")?;
    loop {
        if cancel.is_canceled() {
            return Ok(());
        }
        let now_ns = device
            .configure(|usrp| usrp.get_current_time(CHANNEL))?
            .into_nanos();
        if now_ns < command_time_ns {
            std::thread::sleep(Duration::from_nanos(u64::try_from(
                (command_time_ns - now_ns).min(10_000_000),
            )?));
            continue;
        }
        ensure!(
            now_ns < deadline_ns,
            "RX LO did not lock during survey dwell {dwell_index}"
        );
        let sensor = device.configure(|usrp| usrp.get_rx_sensor(LO_LOCKED_SENSOR, CHANNEL))?;
        match sensor {
            uhd::SensorValue::Boolean(true) => {
                let locked_at = device.configure(|usrp| usrp.get_current_time(CHANNEL))?;
                ensure!(
                    locked_at.into_nanos() < deadline_ns,
                    "RX LO locked after survey dwell {dwell_index} ended"
                );
                events
                    .send(ControllerEvent::LoLocked {
                        dwell_index,
                        at: locked_at,
                    })
                    .context("survey sink disconnected while reporting LO lock")?;
                debug!(
                    "RX LO locked for dwell {dwell_index} after {} us",
                    (locked_at.into_nanos() - command_time_ns) / 1_000
                );
                return Ok(());
            }
            uhd::SensorValue::Boolean(false) => std::thread::sleep(SENSOR_POLL_INTERVAL),
            value => {
                anyhow::bail!("RX sensor {LO_LOCKED_SENSOR:?} returned non-Boolean value {value:?}")
            }
        }
    }
}

/// Calculate a dwell's absolute device start time.
fn dwell_start_ns(start_time_ns: i64, dwell_ns: i64, dwell_index: u64) -> Result<i64> {
    let offset = i64::try_from(dwell_index)?
        .checked_mul(dwell_ns)
        .context("survey duration overflows device time")?;
    start_time_ns
        .checked_add(offset)
        .context("survey device time overflow")
}

/// Convert a positive duration to signed nanoseconds.
fn duration_ns(name: &str, duration: Duration) -> Result<i64> {
    ensure!(!duration.is_zero(), "{name} must be greater than zero");
    i64::try_from(duration.as_nanos()).with_context(|| format!("{name} is too large"))
}

/// Resolve and validate an LO offset that places DC outside the saved band.
fn resolve_lo_offset(requested: Option<f64>, step: f64, sample_rate: f64) -> Result<f64> {
    let retained_edge = step / 2.0;
    let nyquist = sample_rate / 2.0;
    let offset = requested.unwrap_or((retained_edge + nyquist) / 2.0);
    ensure!(offset.is_finite(), "LO offset must be finite");
    if offset == 0.0 {
        return Ok(offset);
    }
    ensure!(
        offset.abs() > retained_edge,
        "absolute LO offset must exceed the retained-band edge {retained_edge} Hz"
    );
    ensure!(
        offset.abs() < nyquist,
        "absolute LO offset must be below Nyquist {nyquist} Hz"
    );
    Ok(offset)
}

/// Reject floating-point values that cannot safely be converted to i64.
fn ensure_i64(value: f64, name: &str) -> rustradio::Result<()> {
    if value.is_finite() && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        Ok(())
    } else {
        Err(Error::msg(format!("{name} does not fit in i64")))
    }
}

/// Map an unshifted FFT bin to its signed baseband frequency.
fn bin_offset(bin: usize, fft_size: usize, sample_rate: f64) -> f64 {
    let first_negative = fft_size.div_ceil(2);
    let signed_bin = if bin >= first_negative {
        bin as isize - fft_size as isize
    } else {
        bin as isize
    };
    signed_bin as f64 * sample_rate / fft_size as f64
}

/// Iterate unshifted FFT indices in ascending baseband-frequency order.
fn shifted_indices(fft_size: usize) -> impl Iterator<Item = usize> {
    let first_negative = fft_size.div_ceil(2);
    (first_negative..fft_size).chain(0..first_negative)
}

fn main() -> Result<()> {
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose as usize)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    ensure!(
        opt.sample_rate.is_finite() && opt.sample_rate > 0.0,
        "sample rate must be positive and finite"
    );
    ensure!(opt.gain.is_finite(), "gain must be finite");
    ensure!(opt.fft_size > 0, "FFT size must be greater than zero");
    let dwell_ns = duration_ns("dwell", opt.dwell)?;
    let settle_ns = duration_ns("settle", opt.settle)?;
    let lead_ns = duration_ns("command lead", opt.command_lead)?;
    ensure!(
        settle_ns
            .checked_add(lead_ns)
            .is_some_and(|guarded_ns| guarded_ns < dwell_ns),
        "settling interval plus command lead must be shorter than dwell"
    );

    let device = UhdDevice::open(&opt.device)?;
    if let Some(clock_source) = &opt.clock_source {
        device.configure(|usrp| usrp.set_clock_source(clock_source, CHANNEL))?;
    }
    if let Some(time_source) = &opt.time_source {
        device.configure(|usrp| usrp.set_time_source(time_source, CHANNEL))?;
    }
    let (sample_rate, device_frequency_start, device_frequency_stop, rx_sensor_names) = device
        .configure(|usrp| {
            usrp.set_rx_sample_rate(opt.sample_rate, CHANNEL)?;
            usrp.set_rx_dc_offset_enabled(true, CHANNEL)?;
            let sample_rate = usrp.get_rx_sample_rate(CHANNEL)?;
            let frequency_range = usrp.get_rx_frequency_range(CHANNEL)?;
            Ok((
                sample_rate,
                frequency_range.start()?,
                frequency_range.stop()?,
                usrp.get_rx_sensor_names(CHANNEL)?,
            ))
        })?;
    ensure!(
        rx_sensor_names.iter().any(|name| name == LO_LOCKED_SENSOR),
        "UHD device does not expose the required RX sensor {LO_LOCKED_SENSOR:?}; available sensors: {rx_sensor_names:?}"
    );
    let step = opt.step.unwrap_or(sample_rate * 0.8);
    let plan = Arc::new(ScanPlan::new(
        opt.start_frequency,
        opt.stop_frequency,
        step,
        sample_rate,
        opt.sweeps,
    )?);
    let lo_offset = resolve_lo_offset(opt.lo_offset, step, sample_rate)?;
    let first_center = plan.bands.first().expect("scan plan is nonempty").center;
    let last_center = plan.bands.last().expect("scan plan is nonempty").center;
    let (first_rf_lo, last_rf_lo) = if opt.fixed_lo_offset {
        (first_center + lo_offset, last_center + lo_offset)
    } else {
        (
            first_center - lo_offset.abs(),
            last_center + lo_offset.abs(),
        )
    };
    ensure!(
        first_rf_lo.min(last_rf_lo) >= device_frequency_start
            && first_rf_lo.max(last_rf_lo) <= device_frequency_stop,
        "requested coverage and LO offset require RF LO frequencies outside the device range {device_frequency_start}..={device_frequency_stop} Hz"
    );
    let usable_samples =
        (dwell_ns - settle_ns - lead_ns) as f64 * sample_rate / NANOS_PER_SECOND as f64;
    ensure!(
        usable_samples >= opt.fft_size as f64,
        "post-settling dwell contains only {usable_samples:.1} samples, fewer than FFT size {}",
        opt.fft_size
    );

    let lo_offset_description = if !opt.fixed_lo_offset && lo_offset != 0.0 {
        format!("±{:.3}", lo_offset.abs() / 1e6)
    } else {
        format!("{:+.3}", lo_offset / 1e6)
    };
    eprintln!(
        "Surveying {:.3}–{:.3} MHz with {} centers at {:.6} MS/s, LO offset {lo_offset_description} MHz",
        opt.start_frequency / 1e6,
        opt.stop_frequency / 1e6,
        plan.bands.len(),
        sample_rate / 1e6,
    );

    let mut graph = MTGraph::new();
    let mut source_builder =
        UhdSource::builder(&device, first_center, opt.sample_rate).gain(opt.gain);
    if let Some(antenna) = &opt.antenna {
        source_builder = source_builder.antenna(antenna);
    }
    let prev = blockchain![graph, prev, source_builder.build()?];

    let now_ns = device
        .configure(|usrp| usrp.get_current_time(CHANNEL))?
        .into_nanos();
    let start_time_ns = now_ns
        .checked_add(lead_ns.checked_mul(2).context("command lead overflow")?)
        .context("survey start time overflow")?;
    let (controller_events_tx, controller_events_rx) = mpsc::channel();
    graph.add(Box::new(SurveySink::new(
        prev,
        &opt.output,
        Arc::clone(&plan),
        SurveyConfig {
            start_time_ns,
            dwell_ns,
            settle_ns,
            command_lead_ns: lead_ns,
            sample_rate,
            fft_size: opt.fft_size,
        },
        SurveyOutputConfig {
            lo_offset,
            alternate_lo_offset: !opt.fixed_lo_offset,
            summarize: opt.summarize,
        },
        controller_events_rx,
    )?));

    let cancel = graph.cancel_token();
    let ctrlc_cancel = cancel.clone();
    ctrlc::set_handler(move || {
        warn!("Got Ctrl-C");
        ctrlc_cancel.cancel();
    })
    .context("failed to set Ctrl-C handler")?;

    let tuner_device = device.clone();
    let tuner_plan = Arc::clone(&plan);
    let tuner_cancel = cancel.clone();
    let tuner = std::thread::spawn(move || {
        let result = run_retune_controller(
            tuner_device,
            tuner_plan,
            RetuneConfig {
                lo_offset,
                alternate_lo_offset: !opt.fixed_lo_offset,
                start_time_ns,
                dwell_ns,
                lead_ns,
            },
            controller_events_tx.clone(),
            tuner_cancel,
        );
        if let Err(error) = &result {
            let _ = controller_events_tx.send(ControllerEvent::Fatal(format!("{error:#}")));
        }
        result
    });

    let graph_result = graph.run();
    cancel.cancel();
    let tuner_result = tuner
        .join()
        .map_err(|_| anyhow::anyhow!("timed retune controller panicked"))?;
    graph_result?;
    tuner_result?;
    if let Some(stats) = graph.generate_stats() {
        eprintln!("{stats}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustradio::blocks::VectorSource;
    use rustradio::stream::Tag;

    #[test]
    fn scan_plan_covers_edges_without_gaps() -> Result<()> {
        let plan = ScanPlan::new(100.0, 110.0, 4.0, 5.0, None)?;
        let centers = plan
            .bands
            .iter()
            .map(|band| band.center)
            .collect::<Vec<_>>();
        let expected = [101.666_666_666_666_67, 105.0, 108.333_333_333_333_33];
        for (actual, expected) in centers.iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-12);
        }
        assert_eq!(plan.bands.first().unwrap().low, 100.0);
        assert_eq!(plan.bands.last().unwrap().high, 110.0);
        for pair in plan.bands.windows(2) {
            assert_eq!(pair[0].high, pair[1].low);
            assert!(!pair[0].includes_high_edge);
        }
        assert!(plan.bands.last().unwrap().includes_high_edge);
        Ok(())
    }

    #[test]
    fn scan_plan_does_not_make_a_short_final_tuning_step() -> Result<()> {
        let plan = ScanPlan::new(2.4e9, 2.5e9, 8.0e6, 10.0e6, None)?;
        assert_eq!(plan.bands.len(), 13);
        let expected_width = 100.0e6 / 13.0;
        for band in &plan.bands {
            assert!((band.high - band.low - expected_width).abs() < 1e-6);
        }
        for pair in plan.bands.windows(2) {
            assert!((pair[1].center - pair[0].center - expected_width).abs() < 1e-6);
        }
        Ok(())
    }

    #[test]
    fn narrow_range_uses_one_center() -> Result<()> {
        let plan = ScanPlan::new(100.0, 103.0, 4.0, 5.0, NonZeroU64::new(2))?;
        assert_eq!(plan.bands.len(), 1);
        assert_eq!(plan.bands[0].center, 101.5);
        assert_eq!(plan.total_dwells, Some(2));
        assert!(plan.finished(2));
        Ok(())
    }

    #[test]
    fn scan_plan_rejects_frequency_gaps() {
        assert!(ScanPlan::new(100.0, 110.0, 6.0, 5.0, None).is_err());
    }

    #[test]
    fn default_lo_offset_uses_guard_between_saved_band_and_nyquist() -> Result<()> {
        assert_eq!(resolve_lo_offset(None, 4.0, 5.0)?, 2.25);
        assert_eq!(resolve_lo_offset(Some(0.0), 4.0, 5.0)?, 0.0);
        assert_eq!(resolve_lo_offset(Some(-2.4), 4.0, 5.0)?, -2.4);
        Ok(())
    }

    #[test]
    fn lo_offset_must_place_dc_in_the_guard_band() {
        assert!(resolve_lo_offset(Some(2.0), 4.0, 5.0).is_err());
        assert!(resolve_lo_offset(Some(2.5), 4.0, 5.0).is_err());
        assert!(resolve_lo_offset(None, 5.0, 5.0).is_err());
    }

    #[test]
    fn lo_offset_alternates_between_complete_sweeps() {
        assert_eq!(lo_offset_for_dwell(4.0, true, 3, 0), 4.0);
        assert_eq!(lo_offset_for_dwell(4.0, true, 3, 2), 4.0);
        assert_eq!(lo_offset_for_dwell(4.0, true, 3, 3), -4.0);
        assert_eq!(lo_offset_for_dwell(4.0, true, 3, 5), -4.0);
        assert_eq!(lo_offset_for_dwell(4.0, true, 3, 6), 4.0);
        assert_eq!(lo_offset_for_dwell(4.0, false, 3, 3), 4.0);
    }

    #[test]
    fn cli_defaults_allow_for_measured_b200_calibration_time() -> Result<()> {
        let opt = Opt::try_parse_from([
            "uhd_rf_survey",
            "--output",
            "survey.txt",
            "--start-frequency",
            "5.15G",
            "--stop-frequency",
            "5.895G",
        ])?;
        assert_eq!(opt.dwell, Duration::from_millis(200));
        assert_eq!(opt.settle, Duration::from_millis(70));
        assert!(!opt.fixed_lo_offset);
        assert!(!opt.summarize);
        Ok(())
    }

    #[test]
    fn cli_accepts_compact_summary_output() -> Result<()> {
        let opt = Opt::try_parse_from([
            "uhd_rf_survey",
            "--output",
            "survey.txt",
            "--start-frequency",
            "2.4G",
            "--stop-frequency",
            "2.5G",
            "--summarize",
        ])?;
        assert!(opt.summarize);
        Ok(())
    }

    #[test]
    fn psd_integrates_to_average_input_power() -> Result<()> {
        let sample_rate = 1024.0;
        let fft_size = 256;
        let mut average = SpectrumAverager::new(sample_rate, fft_size)?;
        for amplitude in [1.0f32, 2.0] {
            for sample in 0..fft_size {
                let phase = 2.0 * std::f32::consts::PI * 17.0 * sample as f32 / fft_size as f32;
                average.push(Complex::from_polar(amplitude, phase));
            }
        }
        let psd = average.finish().unwrap();
        let integrated = psd.iter().sum::<f64>() * sample_rate / fft_size as f64;
        assert!(
            (integrated - 2.5).abs() < 1e-5,
            "integrated PSD was {integrated}"
        );
        Ok(())
    }

    #[test]
    fn psd_does_not_average_muted_frames() -> Result<()> {
        let sample_rate = 1024.0;
        let fft_size = 256;
        let mut average = SpectrumAverager::new(sample_rate, fft_size)?;
        for _ in 0..fft_size {
            average.push(Complex::default());
        }
        for sample in 0..fft_size {
            let phase = 2.0 * std::f32::consts::PI * 17.0 * sample as f32 / fft_size as f32;
            average.push(Complex::from_polar(1.0, phase));
        }
        let psd = average.finish().unwrap();
        let integrated = psd.iter().sum::<f64>() * sample_rate / fft_size as f64;
        assert!(
            (integrated - 1.0).abs() < 1e-5,
            "integrated PSD was {integrated}"
        );
        Ok(())
    }

    #[test]
    fn shifted_bins_are_in_frequency_order() -> Result<()> {
        let offsets = shifted_indices(8)
            .map(|bin| bin_offset(bin, 8, 8.0))
            .collect::<Vec<_>>();
        assert_eq!(offsets, [-4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0]);
        Ok(())
    }

    #[test]
    fn run_summary_matches_two_path_image_rejection() -> Result<()> {
        let plan = ScanPlan::new(100.0, 104.0, 4.0, 4.0, None)?;
        let mut summary = RunSummary::new(&plan, 4.0, 4);

        for (lo_offset, psd) in [
            (4.0, vec![9.0, 16.0, 1.0, 4.0]),
            (4.0, vec![9.0, 4.0, 3.0, 8.0]),
            (-4.0, vec![10.0, 8.0, 5.0, 2.0]),
        ] {
            summary.add(
                &Measurement {
                    dwell_index: 0,
                    timestamp_ns: 0,
                    psd,
                },
                lo_offset,
            );
        }

        let expected = [
            (100.0, 2.0, 3.0),
            (101.0, 2.0, 2.0),
            (102.0, 9.0, 9.0),
            (103.0, 8.0, 8.0),
        ];
        for (bin, (frequency, mean, maximum)) in summary.bins.iter().zip(expected) {
            assert_eq!(bin.frequency, frequency);
            assert_eq!(bin.count(), 3);
            assert_eq!(bin.mean(), mean);
            assert_eq!(bin.maximum(), maximum);
            assert!(bin.uses_image_rejection());
        }
        Ok(())
    }

    #[test]
    fn processor_discards_settling_and_finishes_at_retune() -> Result<()> {
        let plan = Arc::new(ScanPlan::new(100.0, 108.0, 4.0, 4.0, NonZeroU64::new(1))?);
        let mut processor = SurveyProcessor::new(
            plan,
            SurveyConfig {
                start_time_ns: 0,
                dwell_ns: 2_000_000_000,
                settle_ns: 1_000_000_000,
                command_lead_ns: 0,
                sample_rate: 4.0,
                fft_size: 4,
            },
        )?;
        processor.record_lo_lock(0, uhd::TimeSpec::from_nanos(500_000_000))?;
        for time in [0, 250_000_000, 500_000_000, 750_000_000] {
            assert!(
                processor
                    .process_sample(time, Complex::new(9.0, 0.0), false)?
                    .0
                    .is_none()
            );
        }
        for time in [1_000_000_000, 1_250_000_000, 1_500_000_000, 1_750_000_000] {
            assert!(
                processor
                    .process_sample(time, Complex::new(1.0, 0.0), false)?
                    .0
                    .is_none()
            );
        }
        let (measurement, done) =
            processor.process_sample(2_000_000_000, Complex::new(2.0, 0.0), false)?;
        assert!(!done);
        let measurement = measurement.expect("first dwell should complete");
        assert_eq!(measurement.dwell_index, 0);
        assert_eq!(measurement.timestamp_ns, 1_500_000_000);
        assert!(measurement.psd.iter().any(|power| *power > 0.0));
        Ok(())
    }

    #[test]
    fn processor_excludes_the_next_retunes_command_lead() -> Result<()> {
        let plan = Arc::new(ScanPlan::new(100.0, 104.0, 4.0, 4.0, NonZeroU64::new(1))?);
        let mut processor = SurveyProcessor::new(
            plan,
            SurveyConfig {
                start_time_ns: 0,
                dwell_ns: 2_000_000_000,
                settle_ns: 500_000_000,
                command_lead_ns: 500_000_000,
                sample_rate: 4.0,
                fft_size: 2,
            },
        )?;
        processor.record_lo_lock(0, uhd::TimeSpec::default())?;
        for (time_ns, amplitude) in [
            (0, 9.0),
            (250_000_000, 9.0),
            (500_000_000, 1.0),
            (750_000_000, 1.0),
            (1_000_000_000, 1.0),
            (1_250_000_000, 1.0),
            (1_500_000_000, 9.0),
            (1_750_000_000, 9.0),
        ] {
            assert!(
                processor
                    .process_sample(time_ns, Complex::new(amplitude, 0.0), false)?
                    .0
                    .is_none()
            );
        }

        let (measurement, done) =
            processor.process_sample(2_000_000_000, Complex::default(), false)?;
        assert!(done);
        let measurement = measurement.expect("guarded dwell should contain complete frames");
        let integrated = measurement.psd.iter().sum::<f64>() * 4.0 / 2.0;
        assert!((integrated - 1.0).abs() < 1e-5);
        assert_eq!(measurement.timestamp_ns, 1_000_000_000);
        Ok(())
    }

    #[test]
    fn processor_discards_invalid_dwell() -> Result<()> {
        let plan = Arc::new(ScanPlan::new(100.0, 104.0, 4.0, 4.0, NonZeroU64::new(1))?);
        let mut processor = SurveyProcessor::new(
            plan,
            SurveyConfig {
                start_time_ns: 0,
                dwell_ns: 2_000_000_000,
                settle_ns: 1,
                command_lead_ns: 0,
                sample_rate: 4.0,
                fft_size: 2,
            },
        )?;
        processor.record_lo_lock(0, uhd::TimeSpec::from_nanos(0))?;
        processor.process_sample(1, Complex::new(1.0, 0.0), false)?;
        processor.process_sample(250_000_001, Complex::new(1.0, 0.0), true)?;
        let (measurement, done) =
            processor.process_sample(2_000_000_000, Complex::default(), false)?;
        assert!(done);
        assert!(measurement.is_none());
        Ok(())
    }

    #[test]
    fn processor_discards_dwell_without_lo_lock() -> Result<()> {
        let plan = Arc::new(ScanPlan::new(100.0, 104.0, 4.0, 4.0, NonZeroU64::new(1))?);
        let mut processor = SurveyProcessor::new(
            plan,
            SurveyConfig {
                start_time_ns: 0,
                dwell_ns: 2_000_000_000,
                settle_ns: 1,
                command_lead_ns: 0,
                sample_rate: 4.0,
                fft_size: 2,
            },
        )?;
        for time in [1, 250_000_001, 500_000_001, 750_000_001] {
            processor.process_sample(time, Complex::new(1.0, 0.0), false)?;
        }
        let (measurement, done) =
            processor.process_sample(2_000_000_000, Complex::default(), false)?;
        assert!(done);
        assert!(measurement.is_none());
        Ok(())
    }

    #[test]
    fn sink_uses_time_tags_and_writes_completed_dwell() -> Result<()> {
        let samples = vec![Complex::new(1.0, 0.0); 9];
        let tags = [Tag::new(0, TIME_TAG, TagValue::I64(0))];
        let (mut source, stream) = VectorSource::builder(samples).tags(&tags).build()?;
        assert!(matches!(source.work()?, BlockRet::EOF));

        let plan = Arc::new(ScanPlan::new(100.0, 104.0, 4.0, 4.0, NonZeroU64::new(1))?);
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("survey.txt");
        let (controller_events_tx, controller_events_rx) = mpsc::channel();
        controller_events_tx.send(ControllerEvent::LoLocked {
            dwell_index: 0,
            at: uhd::TimeSpec::default(),
        })?;
        let mut sink = SurveySink::new(
            stream,
            &output,
            plan,
            SurveyConfig {
                start_time_ns: 0,
                dwell_ns: 2_000_000_000,
                settle_ns: 1,
                command_lead_ns: 0,
                sample_rate: 4.0,
                fft_size: 2,
            },
            SurveyOutputConfig {
                lo_offset: 0.0,
                alternate_lo_offset: false,
                summarize: false,
            },
            controller_events_rx,
        )?;
        assert!(matches!(sink.work()?, BlockRet::EOF));

        let rows = std::fs::read_to_string(output)?;
        let rows = rows.lines().collect::<Vec<_>>();
        assert_eq!(
            rows[0],
            "# device_time_ns frequency_hz power_dbfs_per_hz lo_offset_hz"
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].split_whitespace().count(), 4);
        assert!(rows[1].ends_with(" 0.000000"));
        assert!(rows[1].starts_with("1000000000 100.000000 "));
        assert!(rows[2].starts_with("1000000000 102.000000 "));
        Ok(())
    }

    #[test]
    fn summary_sink_writes_compact_rows_at_eof() -> Result<()> {
        let samples = vec![Complex::new(1.0, 0.0); 9];
        let tags = [Tag::new(0, TIME_TAG, TagValue::I64(0))];
        let (mut source, stream) = VectorSource::builder(samples).tags(&tags).build()?;
        assert!(matches!(source.work()?, BlockRet::EOF));

        let plan = Arc::new(ScanPlan::new(100.0, 104.0, 4.0, 4.0, NonZeroU64::new(1))?);
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("summary.txt");
        let (controller_events_tx, controller_events_rx) = mpsc::channel();
        controller_events_tx.send(ControllerEvent::LoLocked {
            dwell_index: 0,
            at: uhd::TimeSpec::default(),
        })?;
        let mut sink = SurveySink::new(
            stream,
            &output,
            plan,
            SurveyConfig {
                start_time_ns: 0,
                dwell_ns: 2_000_000_000,
                settle_ns: 1,
                command_lead_ns: 0,
                sample_rate: 4.0,
                fft_size: 2,
            },
            SurveyOutputConfig {
                lo_offset: 0.0,
                alternate_lo_offset: false,
                summarize: true,
            },
            controller_events_rx,
        )?;
        assert!(matches!(sink.work()?, BlockRet::EOF));

        let rows = std::fs::read_to_string(output)?;
        let rows = rows.lines().collect::<Vec<_>>();
        assert_eq!(
            rows[0],
            "# frequency_hz average_power_dbfs_per_hz maximum_power_dbfs_per_hz observations"
        );
        assert!(rows.len() > 1);
        for row in &rows[1..] {
            let fields = row.split_whitespace().collect::<Vec<_>>();
            assert_eq!(fields.len(), 4);
            assert_eq!(fields[3], "1");
        }
        Ok(())
    }

    #[test]
    fn summary_sink_writes_accumulated_rows_when_dropped() -> Result<()> {
        let (_source, stream) = VectorSource::builder(Vec::<Complex>::new()).build()?;
        let plan = Arc::new(ScanPlan::new(100.0, 104.0, 4.0, 4.0, None)?);
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("summary.txt");
        let (_controller_events_tx, controller_events_rx) = mpsc::channel();
        let mut sink = SurveySink::new(
            stream,
            &output,
            plan,
            SurveyConfig {
                start_time_ns: 0,
                dwell_ns: 2_000_000_000,
                settle_ns: 1,
                command_lead_ns: 0,
                sample_rate: 4.0,
                fft_size: 4,
            },
            SurveyOutputConfig {
                lo_offset: 4.0,
                alternate_lo_offset: false,
                summarize: true,
            },
            controller_events_rx,
        )?;
        sink.write_measurement(Measurement {
            dwell_index: 0,
            timestamp_ns: 0,
            psd: vec![9.0, 16.0, 1.0, 4.0],
        })?;
        drop(sink);

        let rows = std::fs::read_to_string(output)?;
        let rows = rows.lines().collect::<Vec<_>>();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1], "100.000000 0.000000000 0.000000000 1");
        assert_eq!(rows[4], "103.000000 12.041199827 12.041199827 1");
        Ok(())
    }

    #[test]
    fn sink_releases_stream_capacity_in_bounded_chunks() -> Result<()> {
        let samples = vec![Complex::new(1.0, 0.0); SURVEY_WORK_SAMPLES + 1];
        let tags = [Tag::new(0, TIME_TAG, TagValue::I64(0))];
        let (mut source, stream) = VectorSource::builder(samples).tags(&tags).build()?;
        assert!(matches!(source.work()?, BlockRet::EOF));

        let plan = Arc::new(ScanPlan::new(100.0, 104.0, 4.0, 4.0, None)?);
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("survey.txt");
        let (_controller_events_tx, controller_events_rx) = mpsc::channel();
        let mut sink = SurveySink::new(
            stream,
            &output,
            plan,
            SurveyConfig {
                start_time_ns: 0,
                dwell_ns: 1_000_000_000_000_000,
                settle_ns: 1,
                command_lead_ns: 0,
                sample_rate: 1.0,
                fft_size: 2,
            },
            SurveyOutputConfig {
                lo_offset: 0.0,
                alternate_lo_offset: false,
                summarize: false,
            },
            controller_events_rx,
        )?;

        assert!(matches!(sink.work()?, BlockRet::Again));
        let (remaining, _) = sink.src.read_buf()?;
        assert_eq!(remaining.len(), 1);
        Ok(())
    }
}
