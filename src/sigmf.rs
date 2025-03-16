//! SigMF implementation.

/*
 * TODO:
 * create sink block.
 * add sigmf archive (tar) support.
 */
use std::io::{Read, Seek, Write};

use anyhow::Result;
use log::debug;
use serde::{Deserialize, Serialize};

const DATATYPE_CF32: &str = "cf32";
const VERSION: &str = "1.1.0";

use crate::block::{Block, BlockRet};
use crate::stream::{ReadStream, WriteStream};
use crate::{Complex, Error, Float, Sample};

/// Capture segment.
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Capture {
    /// Sample index in the dataset file at which this segment takes
    /// effect.
    #[serde(rename = "core:sample_start")]
    core_sample_start: u64,

    /// The index of the sample referenced by `sample_start` relative
    /// to an original sample stream.
    #[serde(rename = "core:global_index", skip_serializing_if = "Option::is_none")]
    core_global_index: Option<u64>,

    /// Header bytes to skip.
    #[serde(rename = "core:header_bytes", skip_serializing_if = "Option::is_none")]
    core_header_bytes: Option<u64>,

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
    core_sample_start: u64,

    /// Annotation width.
    #[serde(rename = "core:sample_count", skip_serializing_if = "Option::is_none")]
    core_sample_count: Option<u64>,

    /// Annotation creator.
    #[serde(rename = "core:generator", skip_serializing_if = "Option::is_none")]
    core_generator: Option<String>,

    /// Annotation label.
    #[serde(rename = "core:label", skip_serializing_if = "Option::is_none")]
    core_label: Option<String>,

    /// Comment.
    #[serde(rename = "core:comment", skip_serializing_if = "Option::is_none")]
    core_comment: Option<String>,

    /// Frequency lower edge.
    #[serde(
        rename = "core:freq_lower_edge",
        skip_serializing_if = "Option::is_none"
    )]
    core_freq_lower_edge: Option<f64>,

    /// Frequency upper edge.
    #[serde(
        rename = "core:freq_upper_edge",
        skip_serializing_if = "Option::is_none"
    )]
    core_freq_upper_edge: Option<f64>,

    /// UUID.
    #[serde(rename = "core:uuid", skip_serializing_if = "Option::is_none")]
    core_uuid: Option<String>,
}

/// Global object.
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Global {
    /// Data format.
    #[serde(rename = "core:datatype")]
    core_datatype: String,

    /// Sample rate.
    #[serde(rename = "core:sample_rate", skip_serializing_if = "Option::is_none")]
    pub core_sample_rate: Option<f64>,

    /// SigMF version.
    #[serde(rename = "core:version")]
    core_version: String,

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
#[derive(Serialize, Deserialize, Debug)]
pub struct SigMF {
    /// Global information.
    pub global: Global,

    /// Capture segments.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<Capture>,

    /// Annotations on the data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

impl SigMF {
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
pub fn write(fname: &str, samp_rate: f64, freq: f64) -> Result<()> {
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
    let serialized = serde_json::to_string(&data).unwrap();

    // Create a file and write the serialized string to it.
    let mut file = std::fs::File::create(fname)?;
    file.write_all(serialized.as_bytes())?;
    Ok(())
}

/// SigMF source builder.
pub struct SigMFSourceBuilder<T: Copy + Type> {
    filename: String,
    // TODO: replace with Repeat::Infinite. Also FileSource.
    repeat: bool,
    sample_rate: Option<f64>,
    dummy: std::marker::PhantomData<T>,
}

impl<T: Default + Copy + Type> SigMFSourceBuilder<T> {
    /// Create new SigMF source builder.
    pub fn new(filename: String) -> Self {
        Self {
            filename,
            repeat: false,
            sample_rate: None,
            dummy: std::marker::PhantomData,
        }
    }
    /// Force a certain sample rate.
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.sample_rate = Some(rate);
        self
    }
    /// Force a certain sample rate.
    pub fn repeat(mut self, repeat: bool) -> Self {
        self.repeat = repeat;
        self
    }
    /// Build a SigMFSource.
    pub fn build(self) -> Result<(SigMFSource<T>, ReadStream<T>)> {
        SigMFSource::new(&self.filename, self.sample_rate, self.repeat)
    }
}

/// SigMF file source.
#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct SigMFSource<T: Copy> {
    file: std::fs::File,
    range: (u64, u64),
    left: u64,
    sample_rate: Option<f64>,
    repeat: bool,
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

impl<T: Default + Copy + Type> SigMFSource<T> {
    /// Create a new SigMF source block.
    ///
    /// If the exact file name exists, then treat it as an Archive.
    /// If it does not, fall back to checking for separate Recording files.
    pub fn new(
        filename: &str,
        samp_rate: Option<f64>,
        repeat: bool,
    ) -> Result<(Self, ReadStream<T>)> {
        if std::fs::exists(filename)? {
            Self::from_archive(filename, samp_rate, repeat)
        } else {
            match Self::from_recording(filename, samp_rate, repeat) {
                Err(e) => Err(Error::new(&format!("SigMF Archive '{filename}' doesn't exist, and trying to read separated Recording files failed too: {e}")).into()),
                Ok(r) => Ok(r),
            }
        }
    }
    /// Create a new SigMF from separated Recording files.
    ///
    fn from_recording(
        base: &str,
        samp_rate: Option<f64>,
        repeat: bool,
    ) -> Result<(Self, ReadStream<T>)> {
        let meta: SigMF = {
            let file = std::fs::File::open(base.to_owned() + "-meta")?;
            let reader = std::io::BufReader::new(file);
            serde_json::from_reader(reader)?
        };
        if let Some(samp_rate) = samp_rate {
            if let Some(t) = meta.global.core_sample_rate {
                if t != samp_rate {
                    return Err(Error::new(&format!(
                        "sigmf file {} sample rate ({}) is not the expected {}",
                        base, t, samp_rate
                    ))
                    .into());
                }
            }
        }
        let file = std::fs::File::open(base.to_owned() + "-data")?;
        let range = (0, file.metadata()?.len());
        let (dst, rx) = crate::stream::new_stream();
        Ok((
            Self {
                file,
                sample_rate: meta.global.core_sample_rate,
                range,
                repeat,
                left: range.1,
                buf: vec![],
                dst,
            },
            rx,
        ))
    }
    /// Create a new SigMF source block.
    fn from_archive(
        filename: &str,
        samp_rate: Option<f64>,
        repeat: bool,
    ) -> Result<(Self, ReadStream<T>)> {
        let (mut file, mut archive) = {
            let file = std::fs::File::open(filename)?;
            let file2 = file.try_clone()?;
            let archive = tar::Archive::new(file);
            (file2, archive)
        };
        let mut found = None;

        // Find the sole metadata.
        for entry in archive.entries_with_seek().unwrap() {
            let mut entry = entry?;
            if entry
                .path()?
                .extension()
                .unwrap_or(std::ffi::OsStr::new(""))
                != "sigmf-meta"
            {
                continue;
            }
            debug!("Tar contents: {:?}", entry.path()?);
            match entry.header().entry_type() {
                tar::EntryType::Regular => {}
                other => {
                    return Err(Error::new(&format!("data file is of bad type {other:?}")).into());
                }
            }
            let mut s = String::new();
            entry.read_to_string(&mut s)?;
            let mut metaname = match entry.path()?.into_owned().into_os_string().into_string() {
                Ok(s) => s,
                Err(s) => {
                    return Err(Error::new(&format!(
                        "failed to convert OsStr '{s:?}' into string"
                    ))
                    .into());
                }
            };
            metaname.truncate(metaname.len() - "-meta".len());
            found = Some(match found {
                Some(_) => {
                    return Err(Error::new(
                        "sigmf doesn't yet support multiple recordings in an archive",
                    )
                    .into());
                }
                None => (metaname, s),
            });
        }
        let (base, meta_string) = match found {
            None => return Err(Error::new("sigmf doesn't contain any recording").into()),
            Some((b, m)) => (b, m),
        };

        // Find the matching data file.
        let want = base + "-data";
        let range = {
            let mut range = None;
            let mut file = file.try_clone()?;
            file.seek(std::io::SeekFrom::Start(0))?;
            let mut archive = tar::Archive::new(file);
            for e in archive.entries_with_seek().unwrap() {
                let e = e.unwrap();
                let got = e
                    .path()
                    .unwrap()
                    .into_owned()
                    .into_os_string()
                    .into_string()
                    .unwrap();
                if got != want {
                    continue;
                }
                match e.header().entry_type() {
                    tar::EntryType::Regular => {}
                    tar::EntryType::GNUSparse => {
                        return Err(Error::new(
                            "SigMF source block doesn't support sparse tar files",
                        )
                        .into());
                    }
                    other => {
                        return Err(
                            Error::new(&format!("data file is of bad type {other:?}")).into()
                        );
                    }
                }
                range = match range {
                    None => Some((e.raw_file_position(), e.size())),
                    Some(_) => {
                        panic!("Multiple files named '{want}' in archive");
                    }
                };
            }
            range
        };
        let range = range.unwrap();
        file.seek(std::io::SeekFrom::Start(range.0))?;
        let meta = parse_meta(&meta_string)?;
        if let Some(samp_rate) = samp_rate {
            if let Some(t) = meta.global.core_sample_rate {
                if t != samp_rate {
                    return Err(Error::new(&format!(
                        "sigmf file {} sample rate ({}) is not the expected {}",
                        filename, t, samp_rate
                    ))
                    .into());
                }
            }
        }
        // TODO: support i8/u8 and _be.
        let expected_type = T::type_string().to_owned() + "_le";
        if meta.global.core_datatype != expected_type {
            return Err(Error::new(&format!(
                "sigmf file {} data type ({}) not the expected {}",
                filename, meta.global.core_datatype, expected_type
            ))
            .into());
        }
        let (dst, rx) = crate::stream::new_stream();
        Ok((
            Self {
                file,
                sample_rate: meta.global.core_sample_rate,
                range,
                repeat,
                left: range.1,
                buf: vec![],
                dst,
            },
            rx,
        ))
    }
    /// Get the sample rate from the meta file.
    pub fn sample_rate(&self) -> Option<f64> {
        self.sample_rate
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
    T: Sample<Type = T> + Copy + std::fmt::Debug + Type,
{
    fn work(&mut self) -> Result<BlockRet, Error> {
        if self.left == 0 {
            if self.repeat {
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
                .map(|d| T::parse(d).unwrap()),
        );
        o.produce(samples, &[]);
        self.buf.drain(..(samples * sample_size));
        Ok(BlockRet::WaitForStream(&self.dst, 1))
    }
}
