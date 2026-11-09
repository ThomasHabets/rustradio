//! Bell 202 modem. Most common used modem for AX.25.
//!
//! This example got its own blog post. See
//! <https://blog.habets.se/2025/06/Software-defined-KISS-modem.html>.
//!
//! ## Examples
//!
//! ### Send/receive APRS
//!
//! 1. Run the modem: `bell202 --freq 144.8m -d driver=uhd --ogain=0.5 --tcp-listen 127.0.0.1:7878`
//! 2. Connect to port 7878 using some app that can talk APRS using KISS.
//!
//! ### Remote SoapySDR, connect to kernel AX.25
//!
//! 1. On the machine with the SDR, run `SoapySDRServer --bind`
//! 2. Start bell202: `bell202 --freq 433.8m -d soapy=0,remote=hostname-here,driver=remote,remote:driver=uhd --ogain 0.5 --tty ./convenience.symlink`
//! 3. Connect to kernel: `kissattach ./convenience.symlink radioname` (set up radioname in /etc/ax25/axports)
//! 4. Disable CRC on kernel KISS: `kissparms -c 1 -p radioname`
//!
//! Kernel stack should now be up and working with bell202 as the modem.
//!
//! ### Audio with Dire Wolf over virtual audio cables
//!
//! Bell202 can also be used in the audio domain. Either with a radio that takes
//! audio (likely an FM radio, which then does the FM-modulation that we do here
//! in software if using an SDR, per above), or for running integration tests
//! with Dire Wolf.
//!
//! Here's how to run against Dire Wolf.
//!
//! On Linux with PipeWire, first create two virtual cables. This uses
//! `pipewire-pulse`, `pactl`, and the ALSA PulseAudio plugin (on Debian/Ubuntu,
//! the plugin is in `libasound2-plugins`). Run these commands in one shell;
//! keep the module IDs for cleanup:
//!
//! ```sh
//! to_direwolf_module=$(pactl load-module module-null-sink \
//!   sink_name=to_direwolf rate=48000 channels=1 channel_map=mono)
//! from_direwolf_module=$(pactl load-module module-null-sink \
//!   sink_name=from_direwolf rate=48000 channels=1 channel_map=mono)
//! ```
//!
//! Each sink's monitor is the other modem's input. Expose all four ends as
//! ALSA devices so both CPAL and Dire Wolf can select them by name. Create
//! this local configuration file; system audio defaults stay unchanged:
//!
//! ```sh
//! cat > bell202-alsa.conf <<'EOF'
//! </usr/share/alsa/alsa.conf>
//! pcm.to-direwolf {
//!   type pulse
//!   device "to_direwolf"
//!   hint { show on description "bell202 output to Dire Wolf" }
//! }
//! pcm.from-direwolf {
//!   type pulse
//!   device "from_direwolf.monitor"
//!   hint { show on description "bell202 input from Dire Wolf" }
//! }
//! pcm.from-bell202 {
//!   type pulse
//!   device "to_direwolf.monitor"
//!   hint { show on description "Dire Wolf input from bell202" }
//! }
//! pcm.to-bell202 {
//!   type pulse
//!   device "from_direwolf"
//!   hint { show on description "Dire Wolf output to bell202" }
//! }
//! EOF
//! cat > bell202-direwolf.conf <<'EOF'
//! ADEVICE from-bell202 to-bell202
//! ARATE 48000
//! ACHANNELS 1
//! CHANNEL 0
//! MYCALL N0CALL
//! MODEM 1200
//! KISSPORT 8001
//! EOF
//! ALSA_CONFIG_PATH="$PWD/bell202-alsa.conf" direwolf -c bell202-direwolf.conf
//! ```
//!
//! In another terminal in the same directory, start bell202. Both programs
//! must run as the same desktop user connected to the same PipeWire server.
//!
//! Build and run without SoapySDR:
//!
//! ```text
//! ALSA_CONFIG_PATH="$PWD/bell202-alsa.conf" \
//! cargo run --release --example bell202 --features audio -- \
//!   --audio --audio-input "from-direwolf" --audio-output "to-direwolf" \
//!   --audio-rate 48000 --tcp-listen 127.0.0.1:7878
//! ```
//!
//! Route bell202 output into Dire Wolf input, and Dire Wolf output into bell202
//! input using two independent virtual audio paths. Configure Dire Wolf with
//! `MODEM 1200` and `ARATE 48000`, and select the opposite endpoints with
//! `ADEVICE`. Device arguments are name substrings; omit them to use the system
//! defaults and route through the audio server. Disable audio processing such
//! as echo cancellation or noise suppression on these paths.
//!
//! Connect a KISS client to bell202's port 7878 and another to Dire Wolf's KISS
//! port (normally 8001). Frames sent by either client travel over audio to the
//! other modem. bell202 supplies the FCS; KISS clients send frames without it.
//! TCP clients share one modem graph. Complete packets from all clients are
//! transmitted in turn, and received packets are broadcast to every client.
//! Slow clients drop received packets rather than stalling other clients.
//! The first client starts the graph; the last disconnect drains transmission
//! and stops it. Connections arriving during drainage wait for a fresh graph.
//! Ctrl-C stops both active and idle sessions.
//!
//! After stopping both programs, remove the virtual cables from the shell
//! where they were created:
//!
//! ```sh
//! pactl unload-module "$to_direwolf_module"
//! pactl unload-module "$from_direwolf_module"
//! ```
//!
//! ### Radio PTT through rigctld
//!
//! Audio mode can key a radio through an existing rigctld TCP server:
//!
//! ```text
//! cargo run --release --example bell202 --features audio -- \
//!   --audio --rigctld 127.0.0.1:4532 \
//!   --ptt-delay-ms 200 --ptt-tail-ms 100 --tcp-listen 127.0.0.1:7878
//! ```
//!
//! Select audio devices connected to the radio, and configure rigctld's radio
//! and PTT method separately. The connection uses rigctld's default non-VFO
//! protocol, verifies each PTT acknowledgement, and starts with PTT off.
//! Key-on precedes audio by `--ptt-delay-ms` (default 100 ms). Key-off follows
//! the audio backend's estimated playback end plus `--ptt-tail-ms` (default
//! 100 ms); increase the tail for additional buffering outside the backend.
//! Queued packets may share one PTT interval. Shutdown and errors attempt to
//! release PTT; network failure can prevent delivery of that command.
use std::io::{Read, Write};

use anyhow::Result;
use clap::Parser;
use log::info;

use rustradio::Float;
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;
use rustradio::stream::ReadStream;
use rustradio::{parse_frequency, parse_verbosity};

#[cfg(any(feature = "audio", test))]
#[path = "bell202/rigctld.rs"]
mod rigctld;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// SoapySDR driver string.
    #[arg(short, required_unless_present = "audio", conflicts_with = "audio")]
    driver: Option<String>,

    /// Use audio input/output instead of an SDR (requires the audio feature).
    #[arg(long)]
    audio: bool,

    /// Input device name substring; defaults to the system input device.
    #[arg(long, requires = "audio")]
    audio_input: Option<String>,

    /// Output device name substring; defaults to the system output device.
    #[arg(long, requires = "audio")]
    audio_output: Option<String>,

    /// Audio device sample rate, in Hz.
    #[arg(long, requires = "audio", default_value_t = 48000, value_parser = clap::value_parser!(u32).range(8000..))]
    audio_rate: u32,

    /// rigctld TCP address for audio-mode PTT; requires its default non-VFO mode.
    #[arg(long, requires = "audio", value_name = "HOST:PORT")]
    rigctld: Option<String>,

    /// Wait after rigctld acknowledges PTT on, before queuing audio.
    #[arg(long, requires = "rigctld", default_value_t = 100)]
    ptt_delay_ms: u64,

    /// Extra PTT hold time after the audio backend's estimated playback end.
    #[arg(long, requires = "rigctld", default_value_t = 100)]
    ptt_tail_ms: u64,

    /// Verbosity level.
    #[arg(short, value_parser=parse_verbosity, default_value = "info")]
    verbose: usize,

    /// TX/RX frequency.
    #[arg(long, value_parser=parse_frequency, required_unless_present = "audio", conflicts_with = "audio")]
    freq: Option<f64>,

    #[arg(long, conflicts_with = "audio", value_parser=parse_frequency, default_value_t = 300000.0)]
    sample_rate: f64,

    /// Output gain. 0.0-1.0.
    #[arg(long, conflicts_with = "audio", default_value_t = 0.0)]
    ogain: f64,

    #[arg(long, default_value = "0.5")]
    symbol_max_deviation: Float,

    #[arg(
        long = "symbol_taps",
        default_value = "0.5,0.5",
        use_value_delimiter = true
    )]
    symbol_taps: Vec<Float>,

    /// Listen for KISS on this address.
    #[arg(long)]
    tcp_listen: Option<String>,

    /// Listen for KISS on a tty. Create symlink to the new TTY for this path.
    #[cfg(feature = "nix")]
    #[arg(long, conflicts_with = "tcp_listen")]
    tty: Option<std::path::PathBuf>,
}

type KissIo = (Box<dyn Write + Send>, Box<dyn Read + Send>);

#[path = "bell202/kiss_hub.rs"]
mod kiss_hub;

// Get a reader and writer for a TTY-backed KISS session.
fn get_kiss_stream(_opt: &Opt) -> Result<KissIo> {
    #[cfg(feature = "nix")]
    {
        use nix::pty::{OpenptyResult, openpty};
        use std::ffi::CStr;
        use std::fs::File;
        use std::os::fd::AsRawFd;
        if let Some(path) = &_opt.tty {
            let OpenptyResult { master, slave } = openpty(None, None)?;
            // SAFETY:
            // Input is from a successful openpty().
            let ptr = unsafe { libc::ptsname(master.as_raw_fd()) };
            if ptr.is_null() {
                return Err(anyhow::Error::msg(
                    "ptsname() on newly created TTY returned NULL",
                ));
            }
            // SAFETY:
            // We have checked for null above.
            let slave_name = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
            info!("Slave tty device: {slave_name}");
            if let Err(e) = std::os::unix::fs::symlink(&slave_name, path) {
                if e.kind() != std::io::ErrorKind::AlreadyExists
                    || !path.symlink_metadata()?.is_symlink()
                {
                    Err(rustradio::Error::file_io(e, path))?;
                }
                log::trace!(
                    "Symlink {} already exists. Attempting to delete it",
                    path.display()
                );
                std::fs::remove_file(path).map_err(|e| rustradio::Error::file_io(e, path))?;
                std::os::unix::fs::symlink(slave_name, path)
                    .map_err(|e| rustradio::Error::file_io(e, path))?;
            }
            let rx = master.try_clone()?;
            std::mem::forget(slave);
            return Ok((Box::new(File::from(master)), Box::new(File::from(rx))));
        }
    }
    Err(anyhow::Error::msg("-tcp or -tty is mandatory"))
}

const BAUD: usize = 1200;

// The modem operates on real AFSK audio; SDR transport adds/removes FM outside
// these shared paths.
fn transmit_audio(
    g: &mut MTGraph,
    input: ReadStream<u8>,
    audio_rate: u32,
) -> Result<ReadStream<Float>> {
    let prev = input;
    Ok(blockchain![
        g,
        prev,
        KissFrame::new(prev),
        KissDecode::new(prev),
        FcsAdder::new(prev),
        HdlcFramer::new(prev),
        PduToStream::new(prev)?,
        NrziEncode::new(prev)?,
        RationalResampler::builder()
            .deci(BAUD)
            .interp(audio_rate as usize)
            .build(prev)?,
        Map::keep_tags(prev, "bits_to_frequency", |s| if s > 0 {
            2200.0 as Float
        } else {
            1200.0
        })?,
        Vco::new(prev, 2.0 * std::f64::consts::PI / audio_rate as f64)?,
        Map::keep_tags(prev, "ComplexToFloat", |s| s.re)?,
        MultiplyConst::new(prev, 0.5)?,
    ])
}

fn receive_audio(
    g: &mut MTGraph,
    input: ReadStream<Float>,
    audio_rate: u32,
    opt: &Opt,
) -> Result<ReadStream<u8>> {
    // Keep the Hilbert transform's window and symbol timing at a fixed rate.
    let rate = 50000.0;
    let center_freq = 1700.0;
    let prev = if audio_rate == rate as u32 {
        input
    } else {
        blockchain![
            g,
            prev,
            RationalResampler::builder()
                .deci(audio_rate as usize)
                .interp(rate as usize)
                .build(input)?,
        ]
    };
    Ok(blockchain![
        g,
        prev,
        Hilbert::new(prev, 65, &rustradio::window::WindowType::Hamming)?,
        QuadratureDemod::new(prev, 1.0)?,
        FftFilterFloat::new(
            prev,
            &rustradio::fir::low_pass(
                rate,
                1100.0,
                100.0,
                &rustradio::window::WindowType::Hamming,
            )?
        )?,
        add_const(prev, -center_freq * 2.0 * std::f32::consts::PI / rate)?,
        SymbolSync::new(
            prev,
            rate / BAUD as Float,
            opt.symbol_max_deviation,
            Box::new(rustradio::symbol_sync::TedZeroCrossing::new()),
            Box::new(rustradio::iir_filter::IirFilter::new(
                opt.symbol_taps.as_slice()
            )?),
        )?,
        BinarySlicer::new(prev)?,
        NrziDecode::new(prev)?,
        HdlcDeframer::new(prev, 10, 1500),
        KissEncode::new(prev),
        PduToStream::new(prev)?,
    ])
}

#[cfg(feature = "audio")]
fn connect_audio(g: &mut MTGraph, opt: &Opt, tx: ReadStream<Float>) -> Result<ReadStream<Float>> {
    let mut sink = AudioSink::builder();
    if let Some(address) = &opt.rigctld {
        let mut rig = rigctld::Rigctld::connect(address)?;
        let lead = std::time::Duration::from_millis(opt.ptt_delay_ms);
        let tail = std::time::Duration::from_millis(opt.ptt_tail_ms);
        sink = sink.transmit_control(move |on| {
            if !on {
                std::thread::sleep(tail);
            }
            rig.set_ptt(on)
                .map_err(|e| rustradio::Error::msg(format!("rigctld PTT: {e:#}")))?;
            if on {
                std::thread::sleep(lead);
            }
            Ok(())
        });
    }
    if let Some(name) = &opt.audio_output {
        sink = sink.device(name);
    }
    let cancel = g.cancel_token();
    // Stop reception only after playback drains and PTT has been released.
    g.add(Box::new(
        sink.on_eof(move || cancel.cancel())
            .build(tx, opt.audio_rate as u64)?,
    ));
    let mut source = AudioSource::builder();
    if let Some(name) = &opt.audio_input {
        source = source.device(name);
    }
    let (source, rx) = source.build(opt.audio_rate as u64)?;
    g.add(Box::new(source));
    Ok(rx)
}

#[cfg(not(feature = "audio"))]
fn connect_audio(
    _g: &mut MTGraph,
    _opt: &Opt,
    _tx: ReadStream<Float>,
) -> Result<ReadStream<Float>> {
    anyhow::bail!("audio mode requires building with --features audio")
}

#[cfg(feature = "soapysdr")]
fn connect_sdr(g: &mut MTGraph, opt: &Opt, tx: ReadStream<Float>) -> Result<ReadStream<Float>> {
    let dev = soapysdr::Device::new(opt.driver.as_deref().unwrap())?;
    let freq = opt.freq.unwrap();
    let prev = tx;
    let prev = blockchain![
        g,
        prev,
        RationalResampler::builder()
            .deci(48000)
            .interp(opt.sample_rate as usize)
            .build(prev)?,
        Vco::new(prev, 2.0 * std::f64::consts::PI * 5000.0 / opt.sample_rate)?,
    ];
    let cancel = g.cancel_token();
    g.add(Box::new(
        SoapySdrSink::builder(&dev, freq, opt.sample_rate)
            .ogain(opt.ogain)?
            .on_eof(move || cancel.cancel())
            .build(prev)?,
    ));
    Ok(blockchain![
        g,
        prev,
        SoapySdrSource::builder(&dev, freq, opt.sample_rate).build()?,
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(
                opt.sample_rate as Float,
                20_000.0,
                100.0,
                &rustradio::window::WindowType::Hamming,
            )?
        )?,
        RationalResampler::builder()
            .deci(opt.sample_rate as usize)
            .interp(50000)
            .build(prev)?,
        QuadratureDemod::new(prev, 1.0)?,
    ])
}

#[cfg(not(feature = "soapysdr"))]
fn connect_sdr(_g: &mut MTGraph, _opt: &Opt, _tx: ReadStream<Float>) -> Result<ReadStream<Float>> {
    anyhow::bail!("SDR mode requires building with --features soapysdr")
}

pub fn main() -> Result<()> {
    println!("bell202 modem");
    let opt = Opt::parse();
    // Report missing features before waiting for a KISS connection.
    anyhow::ensure!(
        !opt.audio || cfg!(feature = "audio"),
        "audio mode requires --features audio"
    );
    anyhow::ensure!(
        opt.audio || cfg!(feature = "soapysdr"),
        "SDR mode requires --features soapysdr"
    );
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .module("soapysdr")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;
    #[cfg(feature = "soapysdr")]
    soapysdr::configure_logging();

    let stop = rustradio::graph::CancellationToken::new();
    let active = std::sync::Arc::new(std::sync::Mutex::new(
        None::<rustradio::graph::CancellationToken>,
    ));
    let signal_stop = stop.clone();
    let signal_active = active.clone();
    ctrlc::set_handler(move || {
        eprintln!("Received Ctrl+C!");
        signal_stop.cancel();
        if let Some(cancel) = signal_active.lock().unwrap().as_ref() {
            cancel.cancel();
        }
    })?;
    let session = |io| {
        let mut graph = MTGraph::new();
        *active.lock().unwrap() = Some(graph.cancel_token());
        if stop.is_canceled() {
            graph.cancel_token().cancel();
        }
        let result = run_session(&opt, io, &mut graph);
        *active.lock().unwrap() = None;
        result
    };
    if let Some(address) = &opt.tcp_listen {
        kiss_hub::serve(
            std::net::TcpListener::bind(address)?,
            &stop,
            session,
            || {
                if let Some(cancel) = active.lock().unwrap().as_ref() {
                    cancel.cancel();
                }
            },
        )
    } else {
        session(get_kiss_stream(&opt)?)
    }
}

fn run_session(opt: &Opt, io: KissIo, g: &mut MTGraph) -> Result<()> {
    info!("Setting up device");
    let (kiss, rx) = io;
    let (source, input) = ReaderSource::new(rx)?;
    g.add(Box::new(source));
    let tx_rate = if opt.audio { opt.audio_rate } else { 48000 };
    let tx = transmit_audio(g, input, tx_rate)?;
    let rx_audio = if opt.audio {
        connect_audio(g, opt, tx)?
    } else {
        connect_sdr(g, opt, tx)?
    };
    let rx_rate = if opt.audio { opt.audio_rate } else { 50000 };
    let output = receive_audio(g, rx_audio, rx_rate, opt)?;
    g.add(Box::new(WriterSink::new(output, kiss)));
    g.run()?;
    println!("{}", g.generate_stats().expect("failed to generate stats"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet() -> Vec<u8> {
        let mut packet: Vec<_> = b"APRS  ".iter().map(|b| b << 1).collect();
        packet.push(0xe0);
        packet.extend(b"N0CALL".iter().map(|b| b << 1));
        packet.extend([0xe1, 0x03, 0xf0]);
        packet.extend(b"bell202 audio interoperability");
        packet
    }

    fn kiss(packet: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xc0, 0x00];
        for &b in packet {
            match b {
                0xc0 => bytes.extend([0xdb, 0xdc]),
                0xdb => bytes.extend([0xdb, 0xdd]),
                _ => bytes.push(b),
            }
        }
        bytes.push(0xc0);
        bytes
    }

    fn modulate(bytes: Vec<u8>, rate: u32) -> Result<Vec<Float>> {
        let mut g = MTGraph::new();
        let (source, input) = VectorSource::new(bytes)?;
        g.add(Box::new(source));
        let audio = transmit_audio(&mut g, input, rate)?;
        let sink = VectorSink::new(audio, 1_000_000);
        let hook = sink.hook();
        g.add(Box::new(sink));
        g.run()?;
        let mut audio = hook.data().samples().to_vec();
        // Flush the receiver's FIR and FFT windows after the final HDLC flag.
        audio.resize(audio.len() + rate as usize / 2, 0.0);
        Ok(audio)
    }

    fn demodulate(audio: Vec<Float>, rate: u32) -> Result<Vec<u8>> {
        let opt = Opt::try_parse_from(["bell202", "--audio"])?;
        let mut g = MTGraph::new();
        let (source, input) = VectorSource::new(audio)?;
        g.add(Box::new(source));
        let bytes = receive_audio(&mut g, input, rate, &opt)?;
        let sink = VectorSink::new(bytes, 10000);
        let hook = sink.hook();
        g.add(Box::new(sink));
        g.run()?;
        let result = hook.data().samples().to_vec();
        Ok(result)
    }

    #[test]
    fn audio_modem_round_trip() -> Result<()> {
        let mut payload = packet();
        payload.extend([0xc0, 0xdb]);
        let bytes = kiss(&payload);
        for rate in [8000, 44100, 48000] {
            let audio = modulate(bytes.clone(), rate)?;
            assert_eq!(demodulate(audio, rate)?, bytes, "sample rate {rate}");
        }
        Ok(())
    }

    #[test]
    fn receives_packets_separated_by_silence() -> Result<()> {
        let bytes = kiss(&packet());
        let mut audio = vec![0.0; 24000];
        let burst = modulate(bytes.clone(), 48000)?;
        audio.extend_from_slice(&burst);
        audio.extend_from_slice(&burst);
        let expected = [bytes.as_slice(), bytes.as_slice()].concat();
        assert_eq!(demodulate(audio, 48000)?, expected);
        Ok(())
    }

    #[test]
    #[ignore = "requires gen_packets and atest from Dire Wolf"]
    fn direwolf_interoperability() -> Result<()> {
        use std::process::Command;

        let dir = tempfile::tempdir()?;
        let wav = dir.path().join("tx.wav");
        let bytes = kiss(&packet());
        let audio = modulate(bytes.clone(), 48000)?;
        let pcm: Vec<_> = audio
            .iter()
            .flat_map(|v| ((v.clamp(-1.0, 1.0) * i16::MAX as Float) as i16).to_le_bytes())
            .collect();
        let len = pcm.len() as u32;
        let mut header = b"RIFF".to_vec();
        header.extend((36 + len).to_le_bytes());
        header.extend(b"WAVEfmt ");
        header.extend(16u32.to_le_bytes());
        header.extend(1u16.to_le_bytes()); // PCM
        header.extend(1u16.to_le_bytes()); // mono
        header.extend(48000u32.to_le_bytes());
        header.extend(96000u32.to_le_bytes()); // bytes per second
        header.extend(2u16.to_le_bytes()); // block alignment
        header.extend(16u16.to_le_bytes());
        header.extend(b"data");
        header.extend(len.to_le_bytes());
        header.extend(pcm);
        std::fs::write(&wav, header)?;
        let decoded = Command::new("atest")
            .args(["-B", "1200", "-L", "1", "-G", "1"])
            .arg(&wav)
            .output()?;
        assert!(
            decoded.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&decoded.stdout),
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert!(
            String::from_utf8_lossy(&decoded.stdout).contains("bell202 audio interoperability")
        );

        let message = dir.path().join("packet.txt");
        std::fs::write(&message, "N0CALL>APRS:bell202 audio interoperability")?;
        let wav = dir.path().join("rx.wav");
        assert!(
            Command::new("gen_packets")
                .args(["-r", "48000", "-o"])
                .arg(&wav)
                .arg(&message)
                .status()?
                .success()
        );
        let wav = std::fs::read(wav)?;
        assert_eq!(&wav[..4], b"RIFF");
        let mut pos = 12;
        let mut audio = Vec::new();
        while pos + 8 <= wav.len() {
            let len = u32::from_le_bytes(wav[pos + 4..pos + 8].try_into()?) as usize;
            let chunk = &wav[pos + 8..pos + 8 + len];
            match &wav[pos..pos + 4] {
                b"fmt " => {
                    assert_eq!(u16::from_le_bytes(chunk[..2].try_into()?), 1);
                    assert_eq!(u16::from_le_bytes(chunk[2..4].try_into()?), 1);
                    assert_eq!(u32::from_le_bytes(chunk[4..8].try_into()?), 48000);
                    assert_eq!(u16::from_le_bytes(chunk[14..16].try_into()?), 16);
                }
                b"data" => audio.extend(
                    chunk
                        .chunks_exact(2)
                        .map(|s| i16::from_le_bytes([s[0], s[1]]) as Float / 32768.0),
                ),
                _ => {}
            }
            pos += 8 + len + len % 2;
        }
        audio.resize(audio.len() + 24000, 0.0);
        assert_eq!(demodulate(audio, 48000)?, bytes);
        Ok(())
    }

    #[test]
    fn audio_and_sdr_arguments_are_mutually_exclusive() {
        assert!(Opt::try_parse_from(["bell202", "--audio"]).is_ok());
        assert!(Opt::try_parse_from(["bell202", "--freq", "144.8m", "-d", "driver=uhd"]).is_ok());
        assert!(Opt::try_parse_from(["bell202", "--audio", "-d", "driver=uhd"]).is_err());
        assert!(Opt::try_parse_from(["bell202", "--audio", "--freq", "144.8m"]).is_err());
        assert!(Opt::try_parse_from(["bell202", "--audio-input", "cable"]).is_err());
        assert!(Opt::try_parse_from(["bell202", "--audio", "--audio-rate", "0"]).is_err());
        assert!(Opt::try_parse_from(["bell202", "--audio", "--rigctld", "localhost:4532"]).is_ok());
        assert!(Opt::try_parse_from(["bell202", "--rigctld", "localhost:4532"]).is_err());
        assert!(Opt::try_parse_from(["bell202", "--audio", "--ptt-delay-ms", "100"]).is_err());
        assert!(Opt::try_parse_from(["bell202", "--audio", "--ptt-tail-ms", "100"]).is_err());
    }
}
