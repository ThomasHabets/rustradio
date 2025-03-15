use std::io::Write;

use anyhow::{Error, Result};
use clap::Parser;

use rustradio::sigmf::{Capture, SigMF};

#[derive(clap::Parser)]
struct Opt {
    /// Sample rate.
    #[arg(long)]
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
    #[arg(long)]
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

    raw: std::path::PathBuf,
}

fn validate_iso8601(s: &str) -> Result<String, String> {
    match chrono::DateTime::parse_from_rfc3339(s) {
        Ok(_) => Ok(s.to_string()),
        Err(e) => Err(format!("Invalid ISO8601 datetime: {}", e)),
    }
}

fn main() -> Result<()> {
    let opt = Opt::parse();
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
        return Err(Error::msg(format!("Data file '{dataname}' already exists")));
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
            Error::msg(format!("Failed to delete meta file '{metaname}': {e2} in the error path for renaming '{:?}' to '{dataname}': {e}", opt.raw)))?;
        return Err(e.into());
    }

    Ok(())
}
