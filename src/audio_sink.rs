use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use log::{debug, error, info, trace};

use crate::block::{Block, BlockRet};
use crate::stream::Streamp;
use crate::{Error, Float};

use std::sync::mpsc::{sync_channel, SyncSender};

struct CpalOutput {
    device: cpal::Device,
    config: cpal::StreamConfig,
}

impl CpalOutput {
    fn new(sample_rate: u32) -> Result<Self> {
        for host in cpal::platform::ALL_HOSTS {
            debug!("Audio sink host: {host:?}, name: {}", host.name());
        }
        let host = cpal::default_host();
        // let host = cpal::host_from_id(cpal::platform::ALL_HOSTS[0])?;
        debug!("Audio sink chose default host {}", host.id().name());
        if false {
            // Printing device names spews a bunch of ALSA errors to stderr.
            // https://github.com/RustAudio/cpal/issues/384
            for dev in host.devices()? {
                debug!("Audio sink device: {:?}", dev.name()?);
            }
        }
        let device = host.default_output_device().ok_or(anyhow::Error::msg(
            "audio sink: failed to find output device",
        ))?;
        info!("Audio sink output device: {}", device.name()?);

        trace!("Audio sink supported output configs:");
        for conf in device.supported_output_configs()? {
            trace!("  {conf:?}");
        }

        let config = device.default_output_config()?;
        debug!("Audio sink using default output config {config:?}");

        let mut config: cpal::StreamConfig = config.into();

        config.sample_rate = cpal::SampleRate(sample_rate);
        config.channels = 1;

        Ok(Self { device, config })
    }

    fn start(&self) -> Result<(SyncSender<f32>, cpal::Stream)> {
        let (sender, receiver) = sync_channel::<f32>(self.config.sample_rate.0 as usize * 3); // 3 seconds buffer

        let channels = self.config.channels as usize;
        let err_fn = |err| error!("an error occurred on stream: {}", err);

        let device = self.device.clone();
        let config = self.config.clone();

        info!("Starting output stream {:?}", config);
        let stream = device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels) {
                    match receiver.recv() {
                        Err(e) => {
                            info!("Failed to read audio samples: {e:?}");
                        }
                        Ok(v) => {
                            let value = f32::from_sample(v);
                            for sample in frame.iter_mut() {
                                *sample = value;
                            }
                        }
                    }
                }
            },
            err_fn,
            None,
        )?;
        stream.play()?;
        Ok((sender, stream))
    }
}

#[derive(rustradio_macros::Block)]
#[rustradio(crate)]
pub struct AudioSink {
    #[rustradio(in)]
    src: Streamp<Float>,
    sender: SyncSender<f32>,

    // Needs to be kept around, but linter thinks it's unused.
    _stream: cpal::Stream,
}

impl AudioSink {
    pub fn new(src: Streamp<Float>, sample_rate: u64) -> Result<Self> {
        let output = CpalOutput::new(sample_rate as u32)?;
        let (sender, stream) = output.start()?;

        Ok(Self {
            src,
            sender,
            _stream: stream,
        })
    }
}

impl Block for AudioSink {
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, _tags) = self.src.read_buf()?;
        let n = i.len();
        for (pos, x) in i.iter().enumerate() {
            if let Err(e) = self.sender.send(*x) {
                i.consume(pos);
                return Err(Error::new(&format!("audio error: {e}")));
            }
        }
        i.consume(n);

        Ok(BlockRet::Noop)
    }
}
