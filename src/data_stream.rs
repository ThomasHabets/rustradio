//! DataStream implementation.
//!
//! Protocol is documented in DATA_STREAM.md. It's an experimental serialized
//! version of a stream meant primarily for use on a websocket between native
//! and a WASM UI.
//!
//! This protocol may be replaced by something more "SDR over network" standard.
//! Though of course for our primary use case it always has to run on top of a
//! websocket.
use std::io::{Read, Write};

use crate::{Error, Result};

/// Current DATA_STREAM.md protocol version.
pub const PROTOCOL_VERSION: u32 = 0;

/// Default maximum payload size accepted by readers.
pub const DEFAULT_MAX_PACKET_LEN: usize = 64 * 1024 * 1024;

// Packet types. 0 is invalid.
const PACKET_VERSION: u8 = 1;
const PACKET_REQUEST_DATA: u8 = 2;
const PACKET_DATA: u8 = 3;

/// Protocol stream identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DataStreamId(String);

impl DataStreamId {
    /// Create a stream identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Return the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return the identifier as bytes.
    #[must_use]
    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Consume the identifier and return the inner string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for DataStreamId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for DataStreamId {
    fn from(id: &str) -> Self {
        Self(id.to_string())
    }
}

impl AsRef<str> for DataStreamId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::borrow::Borrow<str> for DataStreamId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for DataStreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for DataStreamId {
    type Err = std::convert::Infallible;

    fn from_str(id: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self::from(id))
    }
}

/// Request more data for a named stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestData {
    /// Protocol stream identifier.
    pub stream_id: DataStreamId,
    /// Updated receive window, in bytes.
    pub window: usize,
}

impl RequestData {
    /// Create a data request.
    #[must_use]
    pub fn new(stream_id: impl Into<DataStreamId>, window: usize) -> Self {
        Self {
            stream_id: stream_id.into(),
            window,
        }
    }
}

/// Data bytes for a named stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Data {
    /// Protocol stream identifier.
    pub stream_id: DataStreamId,
    /// Data bytes.
    pub data: Vec<u8>,
}

impl Data {
    /// Create a data packet.
    #[must_use]
    pub fn new(stream_id: impl Into<DataStreamId>, data: impl Into<Vec<u8>>) -> Self {
        Self {
            stream_id: stream_id.into(),
            data: data.into(),
        }
    }
}

/// DATA_STREAM.md packet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Packet {
    /// Protocol version.
    Version(u32),

    /// Request more data for a stream.
    RequestData(RequestData),

    /// Data bytes for a stream.
    Data(Data),
}

impl Packet {
    /// Borrow this packet for writing without cloning payloads.
    #[must_use]
    pub fn as_ref(&self) -> PacketRef<'_> {
        match self {
            Packet::Version(version) => PacketRef::Version(*version),
            Packet::RequestData(req) => PacketRef::RequestData {
                stream_id: &req.stream_id,
                window: req.window,
            },
            Packet::Data(data) => PacketRef::Data {
                stream_id: &data.stream_id,
                data: &data.data,
            },
        }
    }
}

/// Borrowed DATA_STREAM.md packet for writing.
///
/// This allows callers to serialize packets with data, without copying the
/// data.
///
/// It has a drawback that every packet type, and its members, kind of need to
/// be implemented twice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacketRef<'a> {
    /// Protocol version.
    Version(u32),

    /// Request more data for a stream.
    RequestData {
        /// Protocol stream identifier.
        stream_id: &'a DataStreamId,
        /// Updated receive window, in bytes.
        window: usize,
    },

    /// Data bytes for a stream.
    Data {
        /// Protocol stream identifier.
        stream_id: &'a DataStreamId,
        /// Data bytes.
        data: &'a [u8],
    },
}

fn as_u32(value: usize, what: &str) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error::msg(format!("{what} does not fit in u32: {value}")))
}

// Return total size of packet, including the packet type, given sections.
fn packet_len(parts: &[usize]) -> Result<u32> {
    let len = parts.iter().try_fold(1usize, |acc, part| {
        acc.checked_add(*part)
            .ok_or_else(|| Error::msg("packet length overflow"))
    })?;
    as_u32(len, "packet length")
}

fn read_raw_packet<R: Read>(reader: &mut R, max_packet_len: usize) -> Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];

    // Dip toe into stream, and if not EOF then read rest of packet.
    match reader.read(&mut len[..1]) {
        Ok(0) => return Ok(None), // This is a clean EOF between packets.
        Ok(1) => reader
            .read_exact(&mut len[1..])
            .map_err(|e| Error::other(e, "reading DataStream header"))?,
        Ok(_) => unreachable!("one byte buffer cannot read more than one byte"),
        Err(e) => return Err(Error::other(e, "reading DataStream first byte")),
    }

    let len = u32::from_le_bytes(len) as usize;
    if len == 0 {
        return Err(Error::msg("packet length must include a packet type byte"));
    }
    if len > max_packet_len {
        return Err(Error::msg(format!(
            "packet length {len} exceeds limit {max_packet_len}"
        )));
    }

    let mut packet = vec![0u8; len];
    reader
        .read_exact(&mut packet)
        .map_err(|e| Error::other(e, "reading DataStream packet"))?;
    Ok(Some(packet))
}

fn parse_packet(packet: &[u8]) -> Result<Packet> {
    if packet.is_empty() {
        return Err(Error::msg("packet payload is empty"));
    }
    let ret = match packet[0] {
        PACKET_VERSION => parse_version(packet).map(Packet::Version),
        PACKET_REQUEST_DATA => parse_request_data(packet).map(Packet::RequestData),
        PACKET_DATA => parse_data(packet).map(Packet::Data),
        other => Err(Error::msg(format!("unsupported packet type {other}"))),
    }?;
    log::trace!("Got packet: {ret:?}");
    Ok(ret)
}

fn parse_version(packet: &[u8]) -> Result<u32> {
    if packet.len() != 5 {
        return Err(Error::msg(format!(
            "version packet has length {}, want 5",
            packet.len()
        )));
    }
    Ok(u32::from_le_bytes(packet[1..5].try_into()?))
}

fn parse_request_data(packet: &[u8]) -> Result<RequestData> {
    // type(1 byte) + window(4 bytes) + stream ID (1+ byte)
    if packet.len() < 6 {
        return Err(Error::msg(format!(
            "RequestData packet has length {}, want at least 6",
            packet.len()
        )));
    }
    Ok(RequestData {
        window: u32::from_le_bytes(packet[1..5].try_into()?) as usize,
        stream_id: String::from_utf8(packet[5..].to_vec())?.into(),
    })
}

fn parse_data(packet: &[u8]) -> Result<Data> {
    // type(1 byte) + window(4 bytes) + stream ID (1+ byte)
    if packet.len() < 6 {
        return Err(Error::msg(format!(
            "Data packet has length {}, want at least 6",
            packet.len()
        )));
    }
    let stream_id_len = u32::from_le_bytes(packet[1..5].try_into()?) as usize;
    let data_start = 5usize
        .checked_add(stream_id_len)
        .ok_or_else(|| Error::msg("Data packet stream ID length overflow"))?;
    if packet.len() < data_start {
        return Err(Error::msg(format!(
            "Data packet stream ID length {stream_id_len} exceeds packet length {}",
            packet.len()
        )));
    }
    Ok(Data {
        stream_id: String::from_utf8(packet[5..data_start].to_vec())?.into(),
        data: packet[data_start..].to_vec(),
    })
}

fn validate_version(packet: Packet) -> Result<()> {
    match packet {
        Packet::Version(PROTOCOL_VERSION) => Ok(()),
        Packet::Version(version) => Err(Error::msg(format!(
            "unsupported protocol version {version}"
        ))),
        other => Err(Error::msg(format!(
            "expected Version packet, got {other:?}"
        ))),
    }
}

/// Return the total buffered frame length when `buf` contains a complete packet.
///
/// The returned length includes the four-byte length prefix. `Ok(None)` means
/// the caller should append more bytes before trying to parse.
fn buffered_packet_len(buf: &[u8], max_packet_len: usize) -> Result<Option<usize>> {
    if buf.len() < 4 {
        return Ok(None);
    }

    let len = u32::from_le_bytes(buf[..4].try_into()?) as usize;
    if len == 0 {
        return Err(Error::msg("packet length must include a packet type byte"));
    }
    if len > max_packet_len {
        return Err(Error::msg(format!(
            "packet length {len} exceeds limit {max_packet_len}"
        )));
    }

    let full_len = 4usize
        .checked_add(len)
        .ok_or_else(|| Error::msg("packet length overflow"))?;
    if buf.len() < full_len {
        return Ok(None);
    }
    Ok(Some(full_len))
}

/// DATA_STREAM.md packet reader for caller-provided byte chunks.
///
/// This is useful when the transport already provides bytes, for example from a
/// websocket frame or a callback-based API. Push any received bytes with
/// [`Self::push_bytes`], then call [`Self::read_packet`] until it returns
/// `Ok(None)`.
pub struct BytesReader {
    buf: Vec<u8>,
    max_packet_len: usize,
}

impl BytesReader {
    /// Create a reader with [`DEFAULT_MAX_PACKET_LEN`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            max_packet_len: DEFAULT_MAX_PACKET_LEN,
        }
    }

    /// Set the maximum accepted packet payload size.
    #[must_use]
    pub fn with_max_packet_len(mut self, max_packet_len: usize) -> Self {
        self.max_packet_len = max_packet_len;
        self
    }

    /// Append transport bytes to the reader.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Return how many bytes are buffered but not yet parsed.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buf.len()
    }

    /// Return true if no bytes are buffered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Discard all buffered bytes.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Parse one packet if a complete frame is buffered.
    ///
    /// Returns `Ok(None)` when more bytes are needed.
    pub fn read_packet(&mut self) -> Result<Option<Packet>> {
        let Some(full_len) = buffered_packet_len(&self.buf, self.max_packet_len)? else {
            return Ok(None);
        };
        let ret = parse_packet(&self.buf[4..full_len]);
        self.buf.drain(..full_len);
        ret.map(Some)
    }

    /// Read and validate the initial version packet if it is buffered.
    ///
    /// Returns `Ok(false)` when more bytes are needed.
    pub fn read_version(&mut self) -> Result<bool> {
        let Some(packet) = self.read_packet()? else {
            return Ok(false);
        };
        validate_version(packet)?;
        Ok(true)
    }
}

impl Default for BytesReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Synchronous DATA_STREAM.md packet reader.
pub struct SyncReader<R> {
    reader: R,
    max_packet_len: usize,
}

impl<R: Read> SyncReader<R> {
    /// Create a reader with [`DEFAULT_MAX_PACKET_LEN`].
    #[must_use]
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            max_packet_len: DEFAULT_MAX_PACKET_LEN,
        }
    }

    /// Set the maximum accepted packet payload size.
    #[must_use]
    pub fn with_max_packet_len(mut self, max_packet_len: usize) -> Self {
        self.max_packet_len = max_packet_len;
        self
    }

    /// Return the wrapped reader.
    #[must_use]
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Read and parse one packet.
    ///
    /// Returns `Ok(None)` only when EOF is reached before a new packet starts.
    pub fn read_packet(&mut self) -> Result<Option<Packet>> {
        read_raw_packet(&mut self.reader, self.max_packet_len)?
            .as_deref()
            .map(parse_packet)
            .transpose()
    }

    /// Read and validate the initial version packet.
    ///
    /// Returns `Ok(false)` if EOF is reached before any packet starts.
    pub fn read_version(&mut self) -> Result<bool> {
        let Some(packet) = self.read_packet()? else {
            return Ok(false);
        };
        validate_version(packet)?;
        Ok(true)
    }
}

/// Synchronous DATA_STREAM.md packet writer.
pub struct SyncWriter<W> {
    writer: W,
}

impl<W: Write> SyncWriter<W> {
    /// Create a writer.
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// Return the wrapped writer.
    #[must_use]
    pub fn into_inner(self) -> W {
        self.writer
    }

    /// Write one packet and flush the wrapped writer.
    pub fn write_packet(&mut self, packet: PacketRef<'_>) -> Result<()> {
        match packet {
            PacketRef::Version(version) => {
                let len = packet_len(&[4])?;
                self.writer.write_all(&len.to_le_bytes())?;
                self.writer.write_all(&[PACKET_VERSION])?;
                self.writer.write_all(&version.to_le_bytes())?;
            }
            PacketRef::RequestData { stream_id, window } => {
                let stream_id = stream_id.as_bytes();
                let len = packet_len(&[4, stream_id.len()])?;
                let window = as_u32(window, "RequestData window")?;
                self.writer.write_all(&len.to_le_bytes())?;
                self.writer.write_all(&[PACKET_REQUEST_DATA])?;
                self.writer.write_all(&window.to_le_bytes())?;
                self.writer.write_all(stream_id)?;
            }
            PacketRef::Data { stream_id, data } => {
                let stream_id = stream_id.as_bytes();
                let len = packet_len(&[4, stream_id.len(), data.len()])?;
                let stream_id_len = as_u32(stream_id.len(), "stream ID length")?;
                self.writer.write_all(&len.to_le_bytes())?;
                self.writer.write_all(&[PACKET_DATA])?;
                self.writer.write_all(&stream_id_len.to_le_bytes())?;
                self.writer.write_all(stream_id)?;
                self.writer.write_all(data)?;
            }
        }
        self.writer.flush()?;
        Ok(())
    }

    /// Write the current protocol version.
    pub fn write_version(&mut self) -> Result<()> {
        self.write_packet(PacketRef::Version(PROTOCOL_VERSION))
    }

    /// Write a RequestData packet.
    pub fn write_request_data(&mut self, stream_id: &DataStreamId, window: usize) -> Result<()> {
        self.write_packet(PacketRef::RequestData { stream_id, window })
    }

    /// Write a Data packet.
    pub fn write_data(&mut self, stream_id: &DataStreamId, data: &[u8]) -> Result<()> {
        self.write_packet(PacketRef::Data { stream_id, data })
    }
}

/// Asynchronous DATA_STREAM.md API.
#[cfg(feature = "async")]
pub mod asynchronous {
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    use super::{
        DEFAULT_MAX_PACKET_LEN, DataStreamId, Error, PACKET_DATA, PACKET_REQUEST_DATA,
        PACKET_VERSION, PROTOCOL_VERSION, Packet, PacketRef, Result, as_u32, packet_len,
        parse_packet, validate_version,
    };

    async fn read_raw_packet<R: AsyncRead + Unpin>(
        reader: &mut R,
        max_packet_len: usize,
    ) -> Result<Option<Vec<u8>>> {
        let mut len = [0u8; 4];

        // Dip toe looking for EOF, then read rest of packet.
        match reader.read(&mut len[..1]).await {
            Ok(0) => return Ok(None), // This is clean EOF.
            Ok(1) => {
                reader
                    .read_exact(&mut len[1..])
                    .await
                    .map_err(|e| Error::other(e, "reading DataStream header"))?;
            }
            Ok(_) => unreachable!("one byte buffer cannot read more than one byte"),
            Err(e) => return Err(Error::other(e, "reading DataStream first byte")),
        }

        let len = u32::from_le_bytes(len) as usize;
        if len == 0 {
            return Err(Error::msg("packet length must include a packet type byte"));
        }
        if len > max_packet_len {
            return Err(Error::msg(format!(
                "packet length {len} exceeds limit {max_packet_len}"
            )));
        }

        let mut packet = vec![0u8; len];
        reader.read_exact(&mut packet).await?;
        Ok(Some(packet))
    }

    /// Asynchronous DATA_STREAM.md packet reader.
    pub struct AsyncReader<R> {
        reader: R,
        max_packet_len: usize,
    }

    impl<R: AsyncRead + Unpin> AsyncReader<R> {
        /// Create a reader with [`DEFAULT_MAX_PACKET_LEN`].
        #[must_use]
        pub fn new(reader: R) -> Self {
            Self {
                reader,
                max_packet_len: DEFAULT_MAX_PACKET_LEN,
            }
        }

        /// Set the maximum accepted packet payload size.
        #[must_use]
        pub fn with_max_packet_len(mut self, max_packet_len: usize) -> Self {
            self.max_packet_len = max_packet_len;
            self
        }

        /// Return the wrapped reader.
        #[must_use]
        pub fn into_inner(self) -> R {
            self.reader
        }

        /// Read and parse one packet.
        ///
        /// Returns `Ok(None)` only when EOF is reached before a new packet starts.
        pub async fn read_packet(&mut self) -> Result<Option<Packet>> {
            read_raw_packet(&mut self.reader, self.max_packet_len)
                .await?
                .as_deref()
                .map(parse_packet)
                .transpose()
        }

        /// Read and validate the initial version packet.
        ///
        /// Returns `Ok(false)` if EOF is reached before any packet starts.
        pub async fn read_version(&mut self) -> Result<bool> {
            let Some(packet) = self.read_packet().await? else {
                return Ok(false);
            };
            validate_version(packet)?;
            Ok(true)
        }
    }

    /// Asynchronous DATA_STREAM.md packet writer.
    pub struct AsyncWriter<W> {
        writer: W,
    }

    impl<W: AsyncWrite + Unpin> AsyncWriter<W> {
        /// Create a writer.
        #[must_use]
        pub fn new(writer: W) -> Self {
            Self { writer }
        }

        /// Return the wrapped writer.
        #[must_use]
        pub fn into_inner(self) -> W {
            self.writer
        }

        /// Write one packet and flush the wrapped writer.
        pub async fn write_packet(&mut self, packet: PacketRef<'_>) -> Result<()> {
            match packet {
                PacketRef::Version(version) => {
                    let len = packet_len(&[4])?;
                    self.writer.write_all(&len.to_le_bytes()).await?;
                    self.writer.write_all(&[PACKET_VERSION]).await?;
                    self.writer.write_all(&version.to_le_bytes()).await?;
                }
                PacketRef::RequestData { stream_id, window } => {
                    let stream_id = stream_id.as_bytes();
                    let len = packet_len(&[4, stream_id.len()])?;
                    let window = as_u32(window, "RequestData window")?;
                    self.writer.write_all(&len.to_le_bytes()).await?;
                    self.writer.write_all(&[PACKET_REQUEST_DATA]).await?;
                    self.writer.write_all(&window.to_le_bytes()).await?;
                    self.writer.write_all(stream_id).await?;
                }
                PacketRef::Data { stream_id, data } => {
                    let stream_id = stream_id.as_bytes();
                    let len = packet_len(&[4, stream_id.len(), data.len()])?;
                    let stream_id_len = as_u32(stream_id.len(), "stream ID length")?;
                    self.writer.write_all(&len.to_le_bytes()).await?;
                    self.writer.write_all(&[PACKET_DATA]).await?;
                    self.writer.write_all(&stream_id_len.to_le_bytes()).await?;
                    self.writer.write_all(stream_id).await?;
                    self.writer.write_all(data).await?;
                }
            }
            self.writer.flush().await?;
            Ok(())
        }

        /// Write the current protocol version.
        pub async fn write_version(&mut self) -> Result<()> {
            self.write_packet(PacketRef::Version(PROTOCOL_VERSION))
                .await
        }

        /// Write a RequestData packet.
        pub async fn write_request_data(
            &mut self,
            stream_id: &DataStreamId,
            window: usize,
        ) -> Result<()> {
            self.write_packet(PacketRef::RequestData { stream_id, window })
                .await
        }

        /// Write a Data packet.
        pub async fn write_data(&mut self, stream_id: &DataStreamId, data: &[u8]) -> Result<()> {
            self.write_packet(PacketRef::Data { stream_id, data }).await
        }
    }
}

#[cfg(feature = "async")]
pub use asynchronous::{AsyncReader, AsyncWriter};

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn sync_roundtrip() -> Result<()> {
        let stream_id = DataStreamId::new("rtl-sdr");
        let mut bytes = Vec::new();
        {
            let mut writer = SyncWriter::new(&mut bytes);
            writer.write_version()?;
            writer.write_request_data(&stream_id, 1234)?;
            writer.write_data(&stream_id, &[1, 2, 3, 4])?;
        }

        let mut reader = SyncReader::new(Cursor::new(bytes));
        assert!(reader.read_version()?);
        assert_eq!(
            reader.read_packet()?,
            Some(Packet::RequestData(RequestData::new("rtl-sdr", 1234)))
        );
        assert_eq!(
            reader.read_packet()?,
            Some(Packet::Data(Data::new("rtl-sdr", vec![1, 2, 3, 4])))
        );
        assert_eq!(reader.read_packet()?, None);
        Ok(())
    }

    #[test]
    fn sync_rejects_non_version_handshake() -> Result<()> {
        let mut bytes = Vec::new();
        SyncWriter::new(&mut bytes).write_request_data(&DataStreamId::new("rtl-sdr"), 1234)?;

        let mut reader = SyncReader::new(Cursor::new(bytes));
        let err = reader.read_version().unwrap_err().to_string();
        assert!(err.contains("expected Version packet"), "{err}");
        Ok(())
    }

    #[test]
    fn bytes_reader_handles_partial_frames() -> Result<()> {
        let stream_id = DataStreamId::new("rtl-sdr");
        let mut bytes = Vec::new();
        {
            let mut writer = SyncWriter::new(&mut bytes);
            writer.write_version()?;
            writer.write_request_data(&stream_id, 1234)?;
            writer.write_data(&stream_id, &[1, 2, 3, 4])?;
        }

        let mut reader = BytesReader::new();
        reader.push_bytes(&bytes[..3]);
        assert!(!reader.read_version()?);
        assert_eq!(reader.buffered_len(), 3);

        reader.push_bytes(&bytes[3..9]);
        assert!(reader.read_version()?);
        assert!(reader.is_empty());

        reader.push_bytes(&bytes[9..12]);
        assert_eq!(reader.read_packet()?, None);
        reader.push_bytes(&bytes[12..]);
        assert_eq!(
            reader.read_packet()?,
            Some(Packet::RequestData(RequestData::new("rtl-sdr", 1234)))
        );
        assert_eq!(
            reader.read_packet()?,
            Some(Packet::Data(Data::new("rtl-sdr", vec![1, 2, 3, 4])))
        );
        assert_eq!(reader.read_packet()?, None);
        assert!(reader.is_empty());
        Ok(())
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn async_roundtrip() -> Result<()> {
        let stream_id = DataStreamId::new("rtl-sdr");
        let (left, right) = tokio::io::duplex(1024);
        let mut writer = AsyncWriter::new(left);
        let mut reader = AsyncReader::new(right);

        writer.write_version().await?;
        assert!(reader.read_version().await?);

        writer.write_request_data(&stream_id, 1234).await?;
        assert_eq!(
            reader.read_packet().await?,
            Some(Packet::RequestData(RequestData::new("rtl-sdr", 1234)))
        );

        writer.write_data(&stream_id, &[1, 2, 3, 4]).await?;
        assert_eq!(
            reader.read_packet().await?,
            Some(Packet::Data(Data::new("rtl-sdr", vec![1, 2, 3, 4])))
        );
        Ok(())
    }
}
