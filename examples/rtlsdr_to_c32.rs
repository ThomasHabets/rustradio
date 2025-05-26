/// Tool to convert from rtlsdr's own format to standard c32 I/Q.
use clap::Parser;

use rustradio::blocks::*;
use rustradio::graph::{Graph, GraphRunner};
use rustradio::{Result, blockchain};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    input: std::path::PathBuf,

    #[arg(short)]
    output: std::path::PathBuf,
}

fn main() -> Result<()> {
    let opt = Opt::parse();

    let mut g = Graph::new();
    let prev = blockchain![
        g,
        prev,
        FileSource::new(opt.input.to_str().unwrap())?,
        RtlSdrDecode::new(prev),
    ];
    g.add(Box::new(FileSink::new(
        prev,
        opt.output,
        rustradio::file_sink::Mode::Overwrite,
    )?));
    g.run()
}
