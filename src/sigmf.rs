//! SigMF implementation.

/*
 * TODO:
 * create sink block.
 * add sigmf archive (tar) support.
 */
use std::io::{Read, Seek, Write};

use log::debug;
use serde::{Deserialize, Serialize};

const DATATYPE_CF32: &str = "cf32";
pub const VERSION: &str = "1.1.0";

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Error, Float, Repeat, Result, Sample};

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::wrap(e, "sigmf")
    }
}

/// Capture segment.
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Capture {
    /// Sample index in the dataset file at which this segment takes
    /// effect.
    #[serde(rename = "core:sample_start")]
    pub core_sample_start: u64,

    /// The index of the sample referenced by `sample_start` relative
    /// to an original sample stream.
    #[serde(rename = "core:global_index", skip_serializing_if = "Option::is_none")]
    pub core_global_index: Option<u64>,

    /// Header bytes to skip.
    #[serde(rename = "core:header_bytes", skip_serializing_if = "Option::is_none")]
    pub core_header_bytes: Option<u64>,

    /// Frequency of capture.
    #[serde(rename = "core:frequency", skip_serializing_if = "Option::is_none")]
    pub core_frequency: Option<f64>,

    /// ISO8601 string for when this was captured.
    #[serde(rename = "core:datetime", skip_serializing_if = "Option::is_none")]
    pub core_datetime: Option<String>,
    // In my example, but not in the spec.
    //#[serde(rename="core:length")]
    //core_length: u64,
}

impl Capture {
    pub fn new(start: u64) -> Self {
        Self {
            core_sample_start: start,
            ..Default::default()
        }
    }
}

/// Annotation segment.
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Annotation {
    /// Sample offset.
    #[serde(rename = "core:sample_start")]
    pub core_sample_start: u64,

    /// Annotation width.
    #[serde(rename = "core:sample_count", skip_serializing_if = "Option::is_none")]
    pub core_sample_count: Option<u64>,

    /// Annotation creator.
    #[serde(rename = "core:generator", skip_serializing_if = "Option::is_none")]
    pub core_generator: Option<String>,

    /// Annotation label.
    #[serde(rename = "core:label", skip_serializing_if = "Option::is_none")]
    pub core_label: Option<String>,

    /// Comment.
    #[serde(rename = "core:comment", skip_serializing_if = "Option::is_none")]
    pub core_comment: Option<String>,

    /// Frequency lower edge.
    #[serde(
        rename = "core:freq_lower_edge",
        skip_serializing_if = "Option::is_none"
    )]
    pub core_freq_lower_edge: Option<f64>,

    /// Frequency upper edge.
    #[serde(
        rename = "core:freq_upper_edge",
        skip_serializing_if = "Option::is_none"
    )]
    pub core_freq_upper_edge: Option<f64>,

    /// UUID.
    #[serde(rename = "core:uuid", skip_serializing_if = "Option::is_none")]
    pub core_uuid: Option<String>,
}

/// Global object.
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Global {
    /// Data format.
    #[serde(rename = "core:datatype")]
    pub core_datatype: String,

    /// Sample rate.
    #[serde(rename = "core:sample_rate", skip_serializing_if = "Option::is_none")]
    pub core_sample_rate: Option<f64>,

    /// SigMF version.
    #[serde(rename = "core:version")]
    pub core_version: String,

    /// Number of channels.
    #[serde(rename = "core:num_channels", skip_serializing_if = "Option::is_none")]
    pub core_num_channels: Option<u64>,

    /// SHA512 of the data.
    #[serde(rename = "core:sha512", skip_serializing_if = "Option::is_none")]
    pub core_sha512: Option<String>,

    // offset
    /// Description.
    #[serde(rename = "core:description", skip_serializing_if = "Option::is_none")]
    pub core_description: Option<String>,

    /// Author of the recording.
    #[serde(rename = "core:author", skip_serializing_if = "Option::is_none")]
    pub core_author: Option<String>,

    // meta_doi
    // data_doi
    /// Recorder software.
    #[serde(rename = "core:recorder", skip_serializing_if = "Option::is_none")]
    pub core_recorder: Option<String>,

    /// License of the data.
    #[serde(rename = "core:license", skip_serializing_if = "Option::is_none")]
    pub core_license: Option<String>,

    /// Hardware used to make the recording.
    #[serde(rename = "core:hw", skip_serializing_if = "Option::is_none")]
    pub core_hw: Option<String>,
    // dataset
    // trailing_bytes
    // metadata_only
    // geolocation
    // extensions
    // collection
}

/// SigMF data.
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct SigMF {
    /// Global information.
    pub global: Global,

    /// Capture segments.
    #[serde()]
    pub captures: Vec<Capture>,

    /// Annotations on the data.
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

impl SigMF {
    /// Create new SigMF object from a data type.
    ///
    /// TODO: Should probably not be done from outside the crate.
    pub fn new(typ: String) -> Self {
        Self {
            global: Global {
                core_version: "1.1.0".to_owned(),
                core_datatype: typ,
                ..Default::default()
            },
            captures: vec![],
            annotations: vec![],
        }
    }
}

/// Parse metadata for SigMF file.
pub fn parse_meta(contents: &str) -> Result<SigMF> {
    Ok(serde_json::from_str(contents)?)
}

/// Write metadata file.
pub fn write<P: AsRef<std::path::Path>>(path: P, samp_rate: f64, freq: f64) -> Result<()> {
    let data = SigMF {
        global: Global {
            core_version: VERSION.to_string(),
            core_datatype: DATATYPE_CF32.to_string(),
            core_sample_rate: Some(samp_rate),
            ..Default::default()
        },
        captures: vec![Capture {
            core_sample_start: 0,
            core_frequency: Some(freq),
            ..Default::default()
        }],
        annotations: Vec::new(),
    };

    // Serialize the data to a JSON string.
    let serialized = serde_json::to_string(&data)?;

    // Create a file and write the serialized string to it.
    let mut file = std::fs::File::create(path)?;
    file.write_all(serialized.as_bytes())?;
    Ok(())
}

/// SigMF source builder.
pub struct SigMFSourceBuilder<T> {
    filename: std::path::PathBuf,
    repeat: Repeat,
    ignore_type_error: bool,
    sample_rate: Option<f64>,
    dummy: std::marker::PhantomData<T>,
}

impl<T: Sample + Type> SigMFSourceBuilder<T> {
    /// Force a certain sample rate.
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.sample_rate = Some(rate);
        self
    }
    /// Force a certain sample rate.
    pub fn repeat(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }
    /// Ignore type error.
    ///
    /// This is used e.g. if you just want the bytes of the data stream, to
    /// checksum or something.
    #[must_use]
    pub fn ignore_type_error(mut self) -> Self {
        self.ignore_type_error = true;
        self
    }
    /// Build a SigMFSource.
    pub fn build(self) -> Result<(SigMFSource<T>, ReadStream<T>)> {
        let mut ret = SigMFSource::new2(&self.filename, self.sample_rate, self.ignore_type_error)?;
        ret.0.repeat = self.repeat;
        Ok(ret)
    }
}

/// SigMF file source.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SigMFSource<T: Sample> {
    file: std::fs::File,
    meta: SigMF,
    range: (u64, u64),
    left: u64,
    repeat: Repeat,
    buf: Vec<u8>,
    #[rustradio(out)]
    dst: WriteStream<T>,
}

/// Trait that needs implementing for all supported SigMF data types.
pub trait Type {
    /// Return full type, or endianness prefix of the type.
    fn type_string() -> &'static str;
}

impl Type for i32 {
    fn type_string() -> &'static str {
        "ri32"
    }
}

impl Type for u8 {
    fn type_string() -> &'static str {
        "ru8"
    }
}

impl Type for num_complex::Complex<i32> {
    fn type_string() -> &'static str {
        "ci32"
    }
}

impl Type for Complex {
    fn type_string() -> &'static str {
        // TODO: support Float being 64bit.
        assert_eq![std::mem::size_of::<Float>(), 4];
        "cf32"
    }
}

impl Type for Float {
    fn type_string() -> &'static str {
        // TODO: support Float being 64bit.
        assert_eq![std::mem::size_of::<Float>(), 4];
        "rf32"
    }
}

fn base_append<P: AsRef<std::path::Path>>(path: P, s: &str) -> std::path::PathBuf {
    let path_ref = path.as_ref();
    let parent = path_ref.parent();
    // "Or default", or return error?
    let filename = path_ref.file_name().unwrap_or_default();
    let mut new_filename = filename.to_os_string();
    new_filename.push(s);
    if let Some(parent) = parent {
        parent.join(new_filename)
    } else {
        std::path::PathBuf::from(new_filename)
    }
}

impl<T: Sample + Type> SigMFSource<T> {
    /// Create new SigMF source builder.
    ///
    /// If the exact file name exists, then treat it as an Archive.
    /// If it does not, fall back to checking for separate Recording files.
    pub fn builder(filename: std::path::PathBuf) -> SigMFSourceBuilder<T> {
        SigMFSourceBuilder {
            filename,
            ignore_type_error: false,
            repeat: Repeat::finite(1),
            sample_rate: None,
            dummy: std::marker::PhantomData,
        }
    }

    /// Create a new SigMF source block.
    ///
    /// If the exact file name exists, then treat it as an Archive.
    /// If it does not, fall back to checking for separate Recording files.
    ///
    /// If samp_rate is provided, and the metadata also provides a sample rate,
    /// then they *must* match, or an error is returned.
    pub fn new<P: AsRef<std::path::Path>>(
        path: P,
        samp_rate: Option<f64>,
    ) -> Result<(Self, ReadStream<T>)> {
        Self::new2(path, samp_rate, false)
    }

    /// Internal creator used by Builder.
    fn new2<P: AsRef<std::path::Path>>(
        path: P,
        samp_rate: Option<f64>,
        ignore_type_error: bool,
    ) -> Result<(Self, ReadStream<T>)> {
        let (block, dst) = if std::fs::exists(&path)? {
            Self::from_archive(&path)?
        } else {
            match Self::from_recording(&path) {
                Err(e) => {
                    return Err(Error::msg(format!(
                        "SigMF Archive '{}' doesn't exist, and trying to read separated Recording files failed too: {e}",
                        path.as_ref().display()
                    )));
                }
                Ok(r) => r,
            }
        };
        let meta = block.meta();
        if let Some(samp_rate) = samp_rate
            && let Some(t) = meta.global.core_sample_rate
            && t != samp_rate
        {
            return Err(Error::msg(format!(
                "sigmf file {} sample rate ({}) is not the expected {}",
                path.as_ref().display(),
                t,
                samp_rate
            )));
        }
        // TODO: support i8/u8 and _be.
        if !ignore_type_error {
            let expected_type = T::type_string().to_owned() + "_le";
            if meta.global.core_datatype != expected_type {
                return Err(Error::msg(format!(
                    "sigmf file {} data type ({}) not the expected {}",
                    path.as_ref().display(),
                    meta.global.core_datatype,
                    expected_type
                )));
            }
        }
        Ok((block, dst))
    }
    /// Create a new SigMF from separated Recording files.
    ///
    fn from_recording<P: AsRef<std::path::Path>>(base: P) -> Result<(Self, ReadStream<T>)> {
        let meta: SigMF = {
            let file = std::fs::File::open(base_append(&base, "-meta"))?;
            let reader = std::io::BufReader::new(file);
            serde_json::from_reader(reader)?
        };
        let file = std::fs::File::open(base_append(base, "-data"))?;
        let range = (0, file.metadata()?.len());
        let (dst, rx) = crate::stream::new_stream();
        Ok((
            Self {
                file,
                meta,
                range,
                repeat: Repeat::finite(1),
                left: range.1,
                buf: vec![],
                dst,
            },
            rx,
        ))
    }
    /// Create a new SigMF source block.
    fn from_archive<P: AsRef<std::path::Path>>(filename: P) -> Result<(Self, ReadStream<T>)> {
        let (mut file, mut archive) = {
            let file = std::fs::File::open(&filename)?;
            let file2 = file.try_clone()?;
            let archive = tar::Archive::new(file);
            (file2, archive)
        };
        let mut found = None;

        // Find the sole metadata.
        for entry in archive.entries_with_seek()? {
            let mut entry = entry?;
            if entry.path()?.extension().unwrap_or_default() != "sigmf-meta" {
                continue;
            }
            debug!("Tar contents: {:?}", entry.path()?);
            match entry.header().entry_type() {
                tar::EntryType::Regular => {}
                other => {
                    return Err(Error::msg(format!("data file is of bad type {other:?}")));
                }
            }
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            let metaname = {
                let mut metaname = entry.path()?.into_owned();
                // Not sure what to do with bad file names. Presumably we can't
                // count on the encoding allowing us to remove "-meta"?
                let new_filename = metaname
                    .file_name()
                    .expect("can't happen: we know it ends in sigmf-meta")
                    .to_str()
                    .ok_or(Error::msg("file name with bad UTF-8?"))?
                    .to_owned();
                let new_filename = &new_filename[..(new_filename.len() - 5)];
                metaname.set_file_name(new_filename);
                metaname
            };
            found = Some(match found {
                Some(_) => {
                    return Err(Error::msg(
                        "sigmf doesn't yet support multiple recordings in an archive",
                    ));
                }
                None => (metaname, s),
            });
        }
        let (base, meta_string) = match found {
            None => return Err(Error::msg("sigmf doesn't contain any recording")),
            Some((b, m)) => (b, m),
        };

        // Find the matching data file.
        let want = base_append(&base, "-data");
        let range = {
            let mut range = None;
            let mut file = file.try_clone()?;
            file.seek(std::io::SeekFrom::Start(0))?;
            let mut archive = tar::Archive::new(file);
            for e in archive.entries_with_seek()? {
                let e = e?;
                let got = e.path()?.into_owned().into_os_string();
                if got != want {
                    continue;
                }
                match e.header().entry_type() {
                    tar::EntryType::Regular => {}
                    tar::EntryType::GNUSparse => {
                        return Err(Error::msg(
                            "SigMF source block doesn't support sparse tar files",
                        ));
                    }
                    other => {
                        return Err(Error::msg(format!("data file is of bad type {other:?}")));
                    }
                }
                range = match range {
                    None => Some((e.raw_file_position(), e.size())),
                    Some(_) => {
                        return Err(Error::msg(format!(
                            "Multiple files named '{}' in archive",
                            want.display()
                        )));
                    }
                };
            }
            range
        };
        let range = range.ok_or(Error::msg(format!(
            "data file for base {} missing",
            base.display()
        )))?;
        file.seek(std::io::SeekFrom::Start(range.0))?;
        let meta = parse_meta(&meta_string)?;
        let (dst, rx) = crate::stream::new_stream();
        Ok((
            Self {
                file,
                meta,
                range,
                repeat: Repeat::finite(1),
                left: range.1,
                buf: vec![],
                dst,
            },
            rx,
        ))
    }
    /// Get the sample rate from the meta file.
    #[must_use]
    pub fn sample_rate(&self) -> Option<f64> {
        self.meta.global.core_sample_rate
    }
    /// Get the SigMF metadata.
    #[must_use]
    pub fn meta(&self) -> &SigMF {
        &self.meta
    }
}

fn u64_to_clamped_usize(v: u64) -> usize {
    if v > (usize::MAX as u64) {
        usize::MAX
    } else {
        v as usize
    }
}

impl<T> Block for SigMFSource<T>
where
    T: Sample<Type = T> + std::fmt::Debug + Type,
{
    fn work(&mut self) -> Result<BlockRet> {
        if self.left == 0 {
            if self.repeat.again() {
                self.file.seek(std::io::SeekFrom::Start(self.range.0))?;
                self.left = self.range.1;
            } else {
                return Ok(BlockRet::EOF);
            }
        }
        let mut o = self.dst.write_buf()?;
        if o.is_empty() {
            return Ok(BlockRet::WaitForStream(&self.dst, 1));
        }
        let sample_size = T::size();
        let have = self.buf.len() / sample_size;
        let want = o.len();
        if have == 0 {
            let left = u64_to_clamped_usize(self.left);
            let want_bytes = std::cmp::min(want * sample_size, left);
            assert_ne!(want_bytes, 0);
            let mut buffer = vec![0; want_bytes];
            let n = self.file.read(&mut buffer)?;
            assert!(n <= left);
            // Can't get EOF here.
            assert_ne!(n, 0);
            self.left -= n as u64;
            self.buf.extend(&buffer[..n]);
        }
        let have = self.buf.len() / sample_size;
        let samples = std::cmp::min(have, want);
        o.fill_from_iter(
            self.buf
                .chunks_exact(sample_size)
                .take(samples)
                .map(|d| T::parse(d).expect("failed to parse a sample")),
        );
        o.produce(samples, &[]);
        self.buf.drain(..(samples * sample_size));
        Ok(BlockRet::WaitForStream(&self.dst, 1))
    }
}
