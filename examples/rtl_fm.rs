/*!
Example broadcast FM receiver, sending output to an Au file.
 */
use std::collections::VecDeque;

use anyhow::Result;
use clap::Parser;
use log::{trace, warn};

use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::{Complex, Float};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Read capture file instead of live from RTL SDR.
    #[arg(short)]
    filename: Option<String>,

    /// File is the output of the rtl-sdr command.
    #[arg(long)]
    rtlsdr_file: bool,

    /// Output file. If unset, use sound card for output.
    #[arg(short)]
    output: Option<std::path::PathBuf>,

    /// Tuned frequency, if reading from RTL SDR.
    #[allow(dead_code)]
    #[arg(long = "freq", default_value = "100000000")]
    freq: u64,

    /// Input gain, if reading from RTL SDR.
    #[allow(dead_code)]
    #[arg(long = "gain", default_value = "20")]
    gain: i32,

    /// Verbosity of debug messages.
    #[arg(short, default_value = "0")]
    verbose: usize,

    /// Audio volume.
    #[arg(long = "volume", default_value = "1.0")]
    volume: Float,

    /// Render frames per second of the UI.
    #[arg(long, default_value_t = 10.0)]
    fps: f32,

    /// Audio output rate.
    #[arg(default_value = "48000")]
    audio_rate: u32,
}

macro_rules! blehbleh {
    ($g:ident, $cons:expr) => {{
        let (block, prev) = $cons;
        $g.add(Box::new(block));
        prev
    }};
}

// TODO: this code is really ugly. It works, but needs major cleanup.
fn run_ui(
    mut terminal: ratatui::DefaultTerminal,
    rx: std::sync::mpsc::Receiver<Float>,
    fps: f32,
) -> Result<()> {
    use crossterm::event::{KeyCode, KeyEventKind};
    let update_rate = std::time::Duration::from_nanos(1_000_000_000u64 / fps as u64);
    let mut paused = false;
    let mut pause_msg = false;
    let mut last_update = std::time::Instant::now();
    const MAX_SIZE: usize = 44100 / 50;
    let mut data: VecDeque<Float> = VecDeque::with_capacity(MAX_SIZE);
    loop {
        while let Ok(s) = rx.try_recv() {
            data.push_back(s);
            if data.len() > MAX_SIZE {
                data.pop_front();
            }
        }
        if !(paused && pause_msg) {
            if last_update.elapsed() > update_rate {
                terminal.clear()?;
                // TODO: why doesn't altscreen remove screen tearing?
                let mut stdout = std::io::stdout();
                crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
                terminal.draw(|frame| render(frame, &data, paused))?;
                if paused {
                    pause_msg = true;
                }
                last_update = std::time::Instant::now();
            }
        }
        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            let event = crossterm::event::read()?;
            match event {
                crossterm::event::Event::Key(key) if key.kind == KeyEventKind::Press => {
                    trace!("Key: {key:?}\r\n");
                    match key.code {
                        KeyCode::Char('q') => break Ok(()),
                        KeyCode::Char('l') => terminal.clear()?,
                        KeyCode::Char(' ') => {
                            paused = !paused;
                            pause_msg = false;
                        }
                        _ => {}
                    };
                }
                _ => {}
            };
        }
    }
}

// Also this code is very ugly.
fn render(frame: &mut ratatui::Frame, data: &VecDeque<Float>, paused: bool) {
    use ratatui::style::Color;
    use ratatui::widgets::canvas::{Canvas, Line};
    use ratatui::widgets::Block;
    let canvas = Canvas::default()
        .block(Block::bordered().title("Audio"))
        .x_bounds([0.0, data.len() as f64])
        .y_bounds([-1.0, 1.0])
        .paint(move |ctx| {
            ctx.layer();
            let mut last = (0.0, 0.0);
            for (n, d) in data.iter().enumerate() {
                let cur = (n as f64, 0.5 * (*d as f64).clamp(-2.0, 2.0));
                ctx.draw(&Line {
                    x1: last.0,
                    y1: last.1,
                    x2: cur.0,
                    y2: cur.1,
                    color: Color::White,
                });
                last = cur;
            }
        });
    frame.render_widget(canvas, frame.area());
    if paused {
        use ratatui::layout::Alignment;
        use ratatui::style::Style;
        use ratatui::widgets::{Borders, Paragraph};
        let msg = Paragraph::new("PAUSED")
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Red)) // Set the text color to red
            .block(Block::default().title("Paused").borders(Borders::ALL));
        frame.render_widget(msg, frame.area());
    }
}

fn main() -> Result<()> {
    println!("rtl_fm receiver example");
    let opt = Opt::parse();
    stderrlog::new()
        .module(module_path!())
        .module("rustradio")
        .quiet(false)
        .verbosity(opt.verbose)
        .timestamp(stderrlog::Timestamp::Second)
        .init()?;

    let (tx, rx) = std::sync::mpsc::channel();

    let pid = std::process::id();
    std::thread::spawn(move || {
        let terminal = ratatui::init();
        let result = run_ui(terminal, rx, opt.fps);
        ratatui::restore();
        result.unwrap();
        unsafe {
            libc::kill(pid as i32, libc::SIGINT);
        }
    });

    let mut g = Graph::new();
    let samp_rate = 1_024_000.0;

    let prev = if let Some(filename) = opt.filename {
        if opt.rtlsdr_file {
            let prev = blehbleh!(g, FileSource::<u8>::new(&filename, false)?);
            blehbleh![g, RtlSdrDecode::new(prev)]
        } else {
            blehbleh!(g, FileSource::<Complex>::new(&filename, false)?)
        }
    } else if !cfg!(feature = "rtlsdr") {
        panic!("RTL SDR feature not enabled")
    } else {
        // RTL SDR source.
        #[cfg(feature = "rtlsdr")]
        {
            let (src, prev) = RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?;
            let (dec, prev) = RtlSdrDecode::new(prev);
            g.add(Box::new(src));
            g.add(Box::new(dec));
            prev
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("can't happen, but must be here to compile")
    };

    // Filter.
    let taps = rustradio::fir::low_pass_complex(
        samp_rate,
        100_000.0,
        1000.0,
        &rustradio::window::WindowType::Hamming,
    );
    let prev = blehbleh![g, FftFilter::new(prev, &taps)];

    // Resample.
    let new_samp_rate = 200_000.0;
    let prev = blehbleh![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];
    let samp_rate = new_samp_rate;

    // TODO: Add broadcast FM deemph.

    // Quad demod.
    let prev = blehbleh![g, QuadratureDemod::new(prev, 1.0)];

    let taps = rustradio::fir::low_pass(
        samp_rate,
        44_100.0,
        500.0,
        &rustradio::window::WindowType::Hamming,
    );
    //let audio_filter = FIRFilter::new(prev, &taps);
    let prev = blehbleh![g, FftFilterFloat::new(prev, &taps)];

    // Resample audio.
    let new_samp_rate = opt.audio_rate as f32;
    let prev = blehbleh![
        g,
        RationalResampler::new(prev, new_samp_rate as usize, samp_rate as usize)?
    ];

    let prev = blehbleh![
        g,
        MapBuilder::new(prev, move |x| {
            if let Err(e) = tx.send(x) {
                trace!("Failed to write data to UI (probably exiting): {e}");
            }
            x
        })
        .name("to_ui".to_owned())
        .build()
    ];

    // Change volume.
    let prev = blehbleh![g, MultiplyConst::new(prev, opt.volume)];

    if let Some(out) = opt.output {
        // Convert to .au.
        let prev = blehbleh![
            g,
            AuEncode::new(
                prev,
                rustradio::au::Encoding::PCM16,
                new_samp_rate as u32,
                1
            )
        ];
        // Save to file.
        g.add(Box::new(FileSink::new(prev, out, Mode::Overwrite)?));
    } else if !cfg!(feature = "audio") {
        panic!("Rustradio build without feature 'audio'. Can only write to file with -o, not play live.");
    } else {
        #[cfg(feature = "audio")]
        {
            // Play live.
            g.add(Box::new(AudioSink::new(prev, new_samp_rate as u64)?));
        }
    }

    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        warn!("Got Ctrl-C");
        eprintln!("\n");
        cancel.cancel();
    })
    .expect("failed to set Ctrl-C handler");
    let st = std::time::Instant::now();
    eprintln!("Running loop");
    g.run()?;
    eprintln!("{}", g.generate_stats(st.elapsed()));
    Ok(())
}
