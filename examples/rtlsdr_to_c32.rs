/// Tool to convert from rtlsdr's own format to standard c32 I/Q.
use clap::Parser;

use rustradio::Result;
use rustradio::blocks::*;
use rustradio::graph::{Graph, GraphRunner};

#[derive(clap::Parser, Debug)]
#[command(version, about)]
struct Opt {
    #[arg(short)]
    input: std::path::PathBuf,

    #[arg(short)]
    output: std::path::PathBuf,
}

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let (block, prev) = $cons;
        $g.add(Box::new(block));
        prev
    }};
}

fn main() -> Result<()> {
    let opt = Opt::parse();

    let mut g = Graph::new();
    let prev = add_block![g, FileSource::<u8>::new(opt.input.to_str().unwrap())?];
    let prev = add_block![g, RtlSdrDecode::new(prev)];
    g.add(Box::new(FileSink::new(
        prev,
        opt.output,
        rustradio::file_sink::Mode::Overwrite,
    )?));
    g.run()
}
