//! SigMF implementation.

/*
 * TODO:
 * create sink block.
 * add sigmf archive (tar) support.
 */
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Write;

const DATATYPE_CF32: &str = "cf32";
const VERSION: &str = "1.1.0";

use crate::block::{Block, BlockRet};
use crate::file_source::FileSource;
use crate::stream::Streamp;
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
    core_frequency: Option<f64>,

    /// ISO8601 string for when this was captured.
    #[serde(rename = "core:datetime", skip_serializing_if = "Option::is_none")]
    core_datetime: Option<String>,
    // In my example, but not in the spec.
    //#[serde(rename="core:length")]
    //core_length: u64,
}

/// Annotation segment.
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Debug)]
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
    core_sample_rate: Option<f64>,

    /// SigMF version.
    #[serde(rename = "core:version")]
    core_version: String,

    /// Number of channels.
    #[serde(rename = "core:num_channels", skip_serializing_if = "Option::is_none")]
    core_num_channels: Option<u64>,

    /// SHA512 of the data.
    #[serde(rename = "core:sha512", skip_serializing_if = "Option::is_none")]
    core_sha512: Option<String>,

    // offset
    /// Description.
    #[serde(rename = "core:description", skip_serializing_if = "Option::is_none")]
    core_description: Option<String>,

    /// Author of the recording.
    #[serde(rename = "core:author", skip_serializing_if = "Option::is_none")]
    core_author: Option<String>,

    // meta_doi
    // data_doi
    /// Recorder software.
    #[serde(rename = "core:recorder", skip_serializing_if = "Option::is_none")]
    core_recorder: Option<String>,

    /// License of the data.
    #[serde(rename = "core:license", skip_serializing_if = "Option::is_none")]
    core_license: Option<String>,

    /// Hardware used to make the recording.
    #[serde(rename = "core:hw", skip_serializing_if = "Option::is_none")]
    core_hw: Option<String>,
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
    global: Global,

    /// Capture segments.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    captures: Vec<Capture>,

    /// Annotations on the data.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    annotations: Vec<Annotation>,
}

/// Parse metadata for SigMF file.
pub fn parse_meta(base: &str) -> Result<SigMF> {
    //let base = "data/1876954_7680KSPS_srsRAN_Project_gnb_short.sigmf";
    let file = std::fs::File::open(format!("{}-meta", base))?;
    let reader = std::io::BufReader::new(file);
    Ok(serde_json::from_reader(reader)?)
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
    sample_rate: Option<f64>,
    dummy: std::marker::PhantomData<T>,
}

impl<T: Default + Copy + Type> SigMFSourceBuilder<T> {
    /// Create new SigMF source builder.
    pub fn new(filename: String) -> Self {
        Self {
            filename,
            sample_rate: None,
            dummy: std::marker::PhantomData,
        }
    }
    /// Force a certain sample rate.
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.sample_rate = Some(rate);
        self
    }
    /// Build a SigMFSource.
    pub fn build(self) -> Result<SigMFSource<T>> {
        SigMFSource::new(&self.filename, self.sample_rate)
    }
}

/// SigMF file source.
pub struct SigMFSource<T: Copy> {
    // TODO: Can't continue to delegate reading the data, because tags.
    file_source: FileSource<T>,
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
    pub fn new(filename: &str, samp_rate: Option<f64>) -> Result<Self> {
        let meta = parse_meta(filename)?;
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
        Ok(Self {
            file_source: FileSource::new(&format!["{}-data", filename], false)?,
        })
    }
    /// Return the output stream.
    pub fn out(&self) -> Streamp<T> {
        self.file_source.out()
    }
}

impl<T> Block for SigMFSource<T>
where
    T: Sample<Type = T> + Copy + std::fmt::Debug + Type,
{
    fn block_name(&self) -> &'static str {
        "SigMFSource"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        self.file_source.work()
    }
}
