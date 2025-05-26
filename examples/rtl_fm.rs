/*!
Example broadcast FM receiver, sending output to an Au file.
 */
use std::collections::VecDeque;

use anyhow::Result;
use clap::Parser;
use log::{trace, warn};

use rustradio::Float;
use rustradio::blockchain;
use rustradio::blocks::*;
use rustradio::file_sink::Mode;
use rustradio::graph::Graph;
use rustradio::graph::GraphRunner;
use rustradio::mtgraph::MTGraph;

const SPECTRUM_SIZE: usize = 1024;

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    /// Read capture file instead of live from RTL SDR.
    #[arg(short)]
    filename: Option<String>,

    /// File is the output of the rtl-sdr command.
    #[arg(long)]
    rtlsdr_file: bool,

    /// Loop the read file forever.
    #[arg(long)]
    file_repeat: bool,

    /// Enable text based UI.
    #[arg(long)]
    tui: bool,

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

    /// Run with multithreaded scheduler.
    #[arg(long)]
    multithread: bool,

    /// Run async graph executor.
    #[arg(long = "async")]
    run_async: bool,
}

// TODO: this code is really ugly. It works, but needs major cleanup.
fn run_ui(
    mut terminal: ratatui::DefaultTerminal,
    rx: std::sync::mpsc::Receiver<Float>,
    rx_spec: std::sync::mpsc::Receiver<Float>,
    fps: f32,
) -> Result<()> {
    use crossterm::event::{KeyCode, KeyEventKind};
    let update_rate = std::time::Duration::from_nanos(1_000_000_000u64 / fps as u64);
    let mut paused = false;
    let mut pause_msg = false;
    let mut last_update = std::time::Instant::now();
    const MAX_SIZE: usize = 44100 / 50;
    let mut data: VecDeque<Float> = VecDeque::with_capacity(MAX_SIZE);
    let mut data_spec: VecDeque<Float> = VecDeque::with_capacity(MAX_SIZE);
    loop {
        loop {
            match rx.try_recv() {
                Ok(s) => {
                    data.push_back(s);
                    if data.len() > MAX_SIZE {
                        data.pop_front();
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        loop {
            match rx_spec.try_recv() {
                Ok(s) => {
                    data_spec.push_back(s);
                    if data_spec.len() > SPECTRUM_SIZE {
                        data_spec.pop_front();
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if !(paused && pause_msg) && last_update.elapsed() > update_rate {
            // Clearing the screen manually creates blinking.
            //terminal.clear()?;

            // TODO: why doesn't altscreen remove screen tearing?
            let mut stdout = std::io::stdout();
            crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
            terminal.draw(|frame| render(frame, &data, &data_spec, paused))?;
            if paused {
                pause_msg = true;
            }
            last_update = std::time::Instant::now();
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
fn render(
    frame: &mut ratatui::Frame,
    data: &VecDeque<Float>,
    data_spec: &VecDeque<Float>,
    paused: bool,
) {
    use ratatui::layout::Constraint::Fill;
    use ratatui::layout::Layout;
    use ratatui::style::Color;
    use ratatui::widgets::Block;
    use ratatui::widgets::canvas::{Canvas, Line};

    let [top, bottom] = Layout::vertical([Fill(1); 2]).areas(frame.area());

    // Draw audio.
    frame.render_widget(
        Canvas::default()
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
            }),
        top,
    );

    // Draw spectrum.
    // TODO: this max value can't simply be sent to a fixed value.
    let max = 40.0;
    frame.render_widget(
        Canvas::default()
            .block(Block::bordered().title("Spectrum"))
            .x_bounds([0.0, data.len() as f64])
            .y_bounds([-10.0, max])
            .paint(move |ctx| {
                ctx.layer();
                let mut last = (0.0, 0.0);
                let rot = data_spec.len() / 2;
                for (n, d) in data_spec
                    .iter()
                    .skip(rot)
                    .chain(data_spec.iter().take(rot))
                    .enumerate()
                {
                    let cur = (n as f64, (*d as f64).clamp(0.0, max));
                    ctx.draw(&Line {
                        x1: last.0,
                        y1: last.1,
                        x2: cur.0,
                        y2: cur.1,
                        color: Color::White,
                    });
                    last = cur;
                }
            }),
        bottom,
    );
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
    if opt.run_async {
        #[cfg(feature = "async")]
        {
            run_async(opt)
        }
        #[cfg(not(feature = "async"))]
        panic!("Async not built in")
    } else {
        run_sync(opt)
    }
}

fn run_sync(opt: Opt) -> Result<()> {
    let mut g: Box<dyn GraphRunner> = if opt.multithread {
        Box::new(MTGraph::new())
    } else {
        Box::new(Graph::new())
    };
    let ui_thread = build(&mut *g, &opt)?;
    let cancel = g.cancel_token();
    ctrlc::set_handler(move || {
        warn!("Got Ctrl-C");
        eprintln!("\n");
        cancel.cancel();
    })
    .expect("failed to set Ctrl-C handler");
    eprintln!("Running loop");
    g.run()?;
    ui_thread.join().expect("Failed to join UI thread");
    eprintln!("{}", g.generate_stats().unwrap());
    Ok(())
}

#[cfg(feature = "async")]
#[tokio::main]
async fn run_async(opt: Opt) -> Result<()> {
    let mut g = rustradio::agraph::AsyncGraph::new();
    let ui_thread = build(&mut g, &opt)?;
    let cancel = g.cancel_token();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for ctrl-C");
        warn!("Got Ctrl-C");
        cancel.cancel();
    });
    eprintln!("Running loop (async)");
    g.run_async().await?;
    ui_thread.join().expect("Failed to join UI thread");
    eprintln!("{}", g.generate_stats().unwrap_or("no stats".to_string()));
    Ok(())
}

fn build(g: &mut dyn GraphRunner, opt: &Opt) -> Result<std::thread::JoinHandle<()>> {
    let (ui_tx, rx) = std::sync::mpsc::channel();
    let (ui_tx_spec, rx_spec) = std::sync::mpsc::channel();

    let pid = std::process::id();
    let opt_tui = opt.tui;
    let opt_fps = opt.fps;
    let ui_thread = std::thread::spawn(move || {
        if opt_tui {
            let terminal = ratatui::init();
            let result = run_ui(terminal, rx, rx_spec, opt_fps);
            ratatui::restore();
            result.unwrap();
            // SAFETY:
            // It's a self-kill. Perfectly safe, I assure you.
            unsafe {
                libc::kill(pid as i32, libc::SIGINT);
            }
        }
    });

    let samp_rate = 1_024_000.0;

    let repeat = if opt.file_repeat {
        rustradio::Repeat::infinite()
    } else {
        rustradio::Repeat::finite(1)
    };
    let prev = if let Some(ref filename) = opt.filename {
        if opt.rtlsdr_file {
            blockchain![
                g,
                prev,
                FileSource::builder(&filename).repeat(repeat).build()?,
                RtlSdrDecode::new(prev),
            ]
        } else {
            blockchain![
                g,
                prev,
                FileSource::builder(&filename).repeat(repeat).build()?,
            ]
        }
    } else if !cfg!(feature = "rtlsdr") {
        panic!("RTL SDR feature not enabled")
    } else {
        // RTL SDR source.
        #[cfg(feature = "rtlsdr")]
        {
            blockchain![
                g,
                prev,
                RtlSdrSource::new(opt.freq, samp_rate as u32, opt.gain)?,
                RtlSdrDecode::new(prev),
            ]
        }
        #[cfg(not(feature = "rtlsdr"))]
        panic!("can't happen, but must be here to compile")
    };

    let (block, spec_tee, prev) = Tee::new(prev);
    g.add(Box::new(block));

    // Send data to spectrum UI.
    {
        let prev = blockchain![
            g,
            prev,
            FftStream::new(spec_tee, SPECTRUM_SIZE),
            Inspect::new(prev, "to_ui_spectrum", move |x, _tags| {
                if opt_tui {
                    if let Err(e) = ui_tx_spec.send(x.norm()) {
                        trace!("Failed to write data to UI (probably exiting): {e}");
                    }
                }
            })
        ];
        g.add(Box::new(NullSink::new(prev)));
    }

    // Resample.
    let samp_rate_2 = 200_000.0;
    let prev = blockchain![
        g,
        prev,
        FftFilter::new(
            prev,
            rustradio::fir::low_pass_complex(
                samp_rate,
                100_000.0,
                1000.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        RationalResampler::builder()
            .deci(samp_rate as usize)
            .interp(samp_rate_2 as usize)
            .build(prev)?,
        QuadratureDemod::new(prev, 1.0),
        FftFilterFloat::new(
            prev,
            &rustradio::fir::low_pass(
                samp_rate_2,
                44_100.0,
                500.0,
                &rustradio::window::WindowType::Hamming,
            )
        ),
        RationalResampler::builder()
            .deci(samp_rate_2 as usize)
            .interp(opt.audio_rate as usize)
            .build(prev)?,
        Inspect::new(prev, "to_ui", move |x, _tags| {
            if opt_tui {
                if let Err(e) = ui_tx.send(x) {
                    trace!("Failed to write data to UI (probably exiting): {e}");
                }
            }
        }),
        MultiplyConst::new(prev, opt.volume),
    ];

    if let Some(ref out) = opt.output {
        // Convert to .au.
        let prev = blockchain![
            g,
            prev,
            AuEncode::new(
                prev,
                rustradio::au::Encoding::Pcm16,
                opt.audio_rate as u32,
                1
            )
        ];
        // Save to file.
        g.add(Box::new(FileSink::new(prev, out, Mode::Overwrite)?));
    } else if !cfg!(feature = "audio") {
        panic!(
            "Rustradio build without feature 'audio'. Can only write to file with -o, not play live."
        );
    } else {
        #[cfg(feature = "audio")]
        {
            // Play live.
            g.add(Box::new(AudioSink::new(prev, opt.audio_rate as u64)?));
        }
    }
    Ok(ui_thread)
}
