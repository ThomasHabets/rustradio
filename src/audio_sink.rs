use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use log::{error, info};

use crate::block::{Block, BlockRet};
use crate::stream::Streamp;
use crate::{Error, Float};

use std::sync::mpsc::{sync_channel, SyncSender};

struct CpalOutput {
    device: cpal::Device,
    config: cpal::StreamConfig,
}

impl CpalOutput {
    fn new(sample_rate: u32) -> Self {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("failed to find output device");
        info!("Output device: {}", device.name().unwrap());

        let config = device.default_output_config().unwrap();

        let mut config: cpal::StreamConfig = config.into();

        config.sample_rate = cpal::SampleRate(sample_rate);
        config.channels = 1;

        Self { device, config }
    }

    fn start(&self) -> SyncSender<f32> {
        let (sender, receiver) = sync_channel::<f32>(self.config.sample_rate.0 as usize * 3); // 3 seconds buffer

        let channels = self.config.channels as usize;
        let err_fn = |err| error!("an error occurred on stream: {}", err);

        let device = self.device.clone();
        let config = self.config.clone();

        std::thread::spawn(move || {
            info!("Starting output stream {:?}", config);
            let stream = device
                .build_output_stream(
                    &config,
                    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                        for frame in data.chunks_mut(channels) {
                            let v = receiver.recv();
                            if v.is_err() {
                                return;
                            }
                            let value = f32::from_sample(v.unwrap());
                            for sample in frame.iter_mut() {
                                *sample = value;
                            }
                        }
                    },
                    err_fn,
                    None,
                )
                .unwrap();
            stream.play().unwrap();

            //wait forever
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1000));
            }
        });

        sender
    }
}

pub struct AudioSink {
    src: Streamp<Float>,
    sender: SyncSender<f32>,
}

impl AudioSink {
    pub fn new(src: Streamp<Float>, sample_rate: u64) -> Self {
        let output = CpalOutput::new(sample_rate as u32);
        let sender = output.start();

        Self { src, sender }
    }
}

impl Block for AudioSink {
    fn block_name(&self) -> &str {
        "AudioSink"
    }
    fn work(&mut self) -> Result<BlockRet, Error> {
        let (i, _tags) = self.src.read_buf()?;
        let n = i.len();
        for x in i.iter() {
            self.sender.send(*x).unwrap();
        }
        i.consume(n);

        Ok(BlockRet::Noop)
    }
}
