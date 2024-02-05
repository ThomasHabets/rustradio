//! SigMF implementation.

use anyhow::Result;
use serde::Deserialize;

/// SigMF file source.
pub struct SigMFSource {}

impl SigMFSource {}

/// Capture segment.
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct Capture {
    /// Sample index in the dataset file at which this segment takes
    /// effect.
    #[serde(rename = "core:sample_start")]
    core_sample_start: u64,

    /// The index of the sample referenced by `sample_start` relative
    /// to an original sample stream.
    #[serde(rename = "core:global_index")]
    core_global_index: Option<u64>,

    /// Header bytes to skip.
    #[serde(rename = "core:header_bytes")]
    core_header_bytes: Option<u64>,

    /// Frequency of capture.
    #[serde(rename = "core:frequency")]
    core_frequency: Option<f64>,

    /// ISO8601 string for when this was captured.
    #[serde(rename = "core:datetime")]
    core_datetime: Option<String>,
    // In my example, but not in the spec.
    //#[serde(rename="core:length")]
    //core_length: u64,
}

/// Annotation segment.
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct Annotation {
    /// Sample offset.
    #[serde(rename = "core:sample_start")]
    core_sample_start: u64,

    /// Annotation width.
    #[serde(rename = "core:sample_count")]
    core_sample_count: Option<u64>,

    /// Annotation creator.
    #[serde(rename = "core:generator")]
    core_generator: Option<String>,

    /// Annotation label.
    #[serde(rename = "core:label")]
    core_label: Option<String>,

    /// Comment.
    #[serde(rename = "core:comment")]
    core_comment: Option<String>,

    /// Frequency lower edge.
    #[serde(rename = "core:freq_lower_edge")]
    core_freq_lower_edge: Option<f64>,

    /// Frequency upper edge.
    #[serde(rename = "core:freq_upper_edge")]
    core_freq_upper_edge: Option<f64>,

    /// UUID.
    #[serde(rename = "core:uuid")]
    core_uuid: Option<String>,
}

/// Global object.
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct Global {
    /// Data format.
    #[serde(rename = "core:datatype")]
    core_datatype: String,

    /// Sample rate.
    #[serde(rename = "core:sample_rate")]
    core_sample_rate: Option<f64>,

    /// SigMF version.
    #[serde(rename = "core:version")]
    core_version: String,

    // num_channels
    /// SHA512 of the data.
    #[serde(rename = "core:sha512")]
    core_sha512: Option<String>,

    // offset
    /// Description.
    #[serde(rename = "core:description")]
    core_description: Option<String>,

    /// Author of the recording.
    #[serde(rename = "core:author")]
    core_author: Option<String>,

    // meta_doi
    // data_doi
    /// Recorder software.
    #[serde(rename = "core:recorder")]
    core_recorder: Option<String>,

    /// License of the data.
    #[serde(rename = "core:license")]
    core_license: Option<String>,

    /// Hardware used to make the recording.
    #[serde(rename = "core:hw")]
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
#[derive(Deserialize, Debug)]
pub struct SigMF {
    /// Global information.
    global: Global,

    /// Capture segments.
    captures: Vec<Capture>,

    /// Annotations on the data.
    annotations: Vec<Annotation>,
}

/// Parse metadata for SigMF file.
pub fn parse_meta() -> Result<SigMF> {
    let base = "data/1876954_7680KSPS_srsRAN_Project_gnb_short.sigmf";
    let file = std::fs::File::open(format!("{}-meta", base))?;
    let reader = std::io::BufReader::new(file);
    let u = serde_json::from_reader(reader)?;
    Ok(u)
}
