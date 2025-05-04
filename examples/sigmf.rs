use std::io::Write;

use anyhow::Result;
use clap::Parser;

use rustradio::Error;
use rustradio::block::{Block, BlockRet};
use rustradio::sigmf::{Capture, SigMF};
use rustradio::stream::NCReadStream;

macro_rules! add_block {
    ($g:ident, $cons:expr) => {{
        let (block, prev) = $cons;
        let block = Box::new(block);
        $g.add(block);
        prev
    }};
}

#[derive(clap::Parser)]
struct Opt {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Args)]
struct CreateOpts {
    /// Sample rate.
    #[arg(long, value_parser = parse_float_with_underscores)]
    sample_rate: f64,

    /// Data type.
    #[arg(long, default_value = "cf32_le")]
    datatype: String,

    /// Capture start time in ISO8601 format.
    ///
    /// YYYY-MM-DDTHH:MM:SS.SSSZ
    #[arg(long, value_parser = validate_iso8601)]
    datetime: Option<String>,

    /// Frequency of capture.
    #[arg(long, value_parser = parse_float_with_underscores)]
    frequency: Option<f64>,

    /// Author.
    #[arg(long)]
    author: Option<String>,

    /// HW.
    #[arg(long)]
    hw: Option<String>,

    /// URL to license.
    #[arg(long)]
    license: Option<String>,

    /// Recorder software.
    #[arg(long)]
    recorder: Option<String>,

    /// Description.
    #[arg(long)]
    description: Option<String>,

    /// SHA512 of the data.
    // TODO: verify format. And allow calculating it.
    #[arg(long)]
    sha512: Option<String>,

    /// Rename base, excluding `.sigmf-{data,meta}`
    #[arg(long)]
    out: String,

    /// Only print metadata. Don't create or rename files.
    #[arg(long)]
    print: bool,

    raw: std::path::PathBuf,
}
#[derive(clap::Subcommand)]
enum Commands {
    /// Create a metadata file for a raw data file, making a Recording.
    Create(CreateOpts),
    /// Parse a SigMF Archive/Recording, and check any checksum.
    Check(CheckOpts),
}

#[derive(clap::Args)]
struct CheckOpts {
    /// Archive or base name for Recording.
    archive: std::path::PathBuf,
}

fn validate_iso8601(s: &str) -> Result<String, String> {
    match chrono::DateTime::parse_from_rfc3339(s) {
        Ok(_) => Ok(s.to_string()),
        Err(e) => Err(format!("Invalid ISO8601 datetime: {}", e)),
    }
}

fn parse_float_with_underscores(s: &str) -> Result<f64, String> {
    use std::str::FromStr;
    let cleaned = s.replace('_', "");
    f64::from_str(&cleaned).map_err(|e| format!("Invalid float: {}", e))
}

fn main() -> Result<()> {
    let opt = Opt::parse();
    match opt.command {
        Commands::Create(opt) => cmd_create(opt),
        Commands::Check(opt) => cmd_check(opt),
    }
}

use rustradio::block;
use rustradio::rustradio_macros;
#[derive(rustradio_macros::Block)]
#[rustradio(crate, new)]
pub struct CheckHash {
    #[rustradio(in)]
    src: NCReadStream<Vec<u8>>,

    correct: String,
}

impl Block for CheckHash {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (v, _tags) = match self.src.pop() {
            None => return Ok(BlockRet::WaitForStream(&self.src, 1)),
            Some(x) => x,
        };
        assert_eq!(
            v.iter().map(|v| format!("{v:02x}")).collect::<String>(),
            self.correct
        );
        println!("Hash is correct!");
        Ok(BlockRet::EOF)
    }
}

fn cmd_check(opt: CheckOpts) -> Result<()> {
    use rustradio::blocks::*;
    use rustradio::graph::GraphRunner;
    let mut g = rustradio::mtgraph::MTGraph::new();
    let src = SigMFSourceBuilder::<u8>::new(opt.archive)
        .ignore_type_error()
        .build()?;
    let Some(ref in_meta) = src.0.meta().global.core_sha512 else {
        eprintln!("Metadata file doesn't have sha512. Nothing to check");
        return Ok(());
    };
    let in_meta = in_meta.to_owned();
    let prev = add_block![g, src];
    let prev = add_block![g, sha512(prev)];
    g.add(Box::new(CheckHash::new(prev, in_meta)));
    g.run().map_err(Into::into)
}

fn cmd_create(opt: CreateOpts) -> Result<()> {
    let mut sigmf = SigMF::new(opt.datatype.clone());
    sigmf.global.core_sample_rate = Some(opt.sample_rate);
    sigmf.global.core_author = opt.author;
    sigmf.global.core_hw = opt.hw;
    sigmf.global.core_license = opt.license;
    sigmf.global.core_recorder = opt.recorder;
    sigmf.global.core_description = opt.description;
    sigmf.global.core_sha512 = opt.sha512;
    let mut cap = Capture::new(0);
    cap.core_frequency = opt.frequency;
    cap.core_datetime = opt.datetime;
    sigmf.captures.push(cap);
    let ser = serde_json::to_string(&sigmf)?;

    let dataname = opt.out.clone() + ".sigmf-data";
    let metaname = opt.out.clone() + ".sigmf-meta";

    if std::path::Path::new(&dataname).exists() {
        return Err(anyhow::Error::msg(format!(
            "Data file '{dataname}' already exists"
        )));
    }

    if opt.print {
        println!("{}", ser);
        return Ok(());
    }
    {
        let mut meta = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&metaname)
            .map_err(|e| Error::msg(format!("Failed to create {metaname}: {e}")))?;
        meta.write_all(ser.as_bytes())?;
        meta.flush()?;
    }

    if let Err(e) = std::fs::rename(&opt.raw, &dataname) {
        std::fs::remove_file(&metaname).map_err(|e2|
            anyhow::Error::msg(format!("Failed to delete meta file '{metaname}': {e2} in the error path for renaming '{:?}' to '{dataname}': {e}", opt.raw)))?;
        return Err(e.into());
    }

    Ok(())
}
