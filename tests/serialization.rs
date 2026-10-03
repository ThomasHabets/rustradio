//! Verify append serialization and its use by stream sinks.
use std::io::Write;
use std::sync::{Arc, Mutex};

use rustradio::block::{Block, BlockRet};
use rustradio::file_sink::{FileSink, Mode};
use rustradio::stream::new_stream;
use rustradio::writer_sink::WriterSink;
use rustradio::{Complex, Float, Result, Sample};

#[test]
fn builtins_append_little_endian_bytes_without_reallocating() {
    let mut out = Vec::with_capacity(128);
    out.push(0xaa);
    let allocation = out.as_ptr();
    let mut expected = vec![0xaa];
    for sample in [0u8, 255] {
        sample.serialize_into(&mut out);
        expected.push(sample);
    }
    for sample in [0u32, 0x12345678, u32::MAX] {
        sample.serialize_into(&mut out);
        expected.extend_from_slice(&sample.to_le_bytes());
    }
    for sample in [i32::MIN, -1, i32::MAX] {
        sample.serialize_into(&mut out);
        expected.extend_from_slice(&sample.to_le_bytes());
    }
    for sample in [
        0.0 as Float,
        -0.0,
        Float::INFINITY,
        Float::from_bits(0x7fc01234),
    ] {
        sample.serialize_into(&mut out);
        expected.extend_from_slice(&sample.to_le_bytes());
    }
    let sample = Complex::new(-1.25, 3.5);
    sample.serialize_into(&mut out);
    expected.extend_from_slice(&sample.re.to_le_bytes());
    expected.extend_from_slice(&sample.im.to_le_bytes());
    assert_eq!(out, expected);
    assert_eq!(out.as_ptr(), allocation);
    assert_eq!(
        sample.serialize(),
        expected[expected.len() - Complex::size()..]
    );
}

#[derive(Debug, Copy, Clone, Default)]
struct Legacy(u8);

impl Sample for Legacy {
    type Type = Self;
    fn size() -> usize {
        1
    }
    fn parse(bytes: &[u8]) -> Result<Self> {
        Ok(Self(bytes[0]))
    }
    fn serialize(&self) -> Vec<u8> {
        vec![self.0]
    }
}

#[test]
fn custom_sample_keeps_legacy_serialization() {
    let mut out = vec![1, 2];
    Legacy(3).serialize_into(&mut out);
    assert_eq!(out, [1, 2, 3]);
}

#[derive(Debug, Copy, Clone, Default)]
struct AppendOnly(u8);

impl Sample for AppendOnly {
    type Type = Self;
    fn size() -> usize {
        1
    }
    fn parse(bytes: &[u8]) -> Result<Self> {
        Ok(Self(bytes[0]))
    }
    fn serialize(&self) -> Vec<u8> {
        panic!("sink must use serialize_into")
    }
    fn serialize_into(&self, out: &mut Vec<u8>) {
        out.push(self.0);
    }
}

#[test]
fn file_sink_serializes_multiple_batches() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("samples");
    let (writer, reader) = new_stream();
    let mut sink = FileSink::new(reader, &path, Mode::Create)?;
    for samples in [
        [AppendOnly(1), AppendOnly(2)],
        [AppendOnly(3), AppendOnly(4)],
    ] {
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&samples);
        window.produce(samples.len(), &[]);
        assert!(matches!(sink.work()?, BlockRet::Again));
    }
    drop(sink);
    assert_eq!(std::fs::read(path)?, [1, 2, 3, 4]);
    Ok(())
}

struct ShortWriter(Arc<Mutex<Vec<u8>>>);

impl Write for ShortWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = bytes.len().min(2);
        self.0.lock().unwrap().extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn writer_sink_serializes_and_drains_partial_writes() -> Result<()> {
    let output = Arc::new(Mutex::new(Vec::new()));
    let (writer, reader) = new_stream();
    let mut sink = WriterSink::new(reader, ShortWriter(output.clone()));
    for batch in [[1, 2, 3, 4, 5], [6, 7, 8, 9, 10]] {
        let samples = batch.map(AppendOnly);
        let mut window = writer.write_buf()?;
        window.fill_from_slice(&samples);
        window.produce(samples.len(), &[]);
        assert!(matches!(sink.work()?, BlockRet::Pending));
        assert!(matches!(sink.work()?, BlockRet::Pending));
        assert!(matches!(sink.work()?, BlockRet::WaitForStream(_, 1)));
    }
    assert_eq!(*output.lock().unwrap(), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    Ok(())
}

#[test]
fn message_sinks_use_append_serialization() -> Result<()> {
    use rustradio::file_sink::NoCopyFileSink;
    use rustradio::pdu_writer::PduWriter;
    use rustradio::stream::new_nocopy_stream;

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("samples");
    let (writer, reader) = new_nocopy_stream();
    let mut sink = NoCopyFileSink::new(reader, &path, Mode::Create)?;
    for value in [1, 2] {
        writer.push(AppendOnly(value), vec![]);
        assert!(matches!(sink.work()?, BlockRet::Again));
    }
    drop(sink);
    assert_eq!(std::fs::read(path)?, [1, 10, 2, 10]);

    let dir = tempfile::tempdir()?;
    let (writer, reader) = new_nocopy_stream();
    let mut sink = PduWriter::new(reader, dir.path());
    writer.push(vec![AppendOnly(3), AppendOnly(4)], vec![]);
    assert!(matches!(sink.work()?, BlockRet::WaitForStream(_, 1)));
    let files = std::fs::read_dir(dir.path())?.collect::<std::io::Result<Vec<_>>>()?;
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(files[0].path())?, [3, 4]);
    Ok(())
}
