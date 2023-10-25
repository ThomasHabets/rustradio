/*! Test program for whole packet clock recovery.

Just a test, for now. Will be turned into a block.

* <https://youtu.be/rQkBDMeODHc>
*/
use std::fs::File;
use std::io::Read;

use anyhow::Result;
use rustfft::FftPlanner;

use rustradio::{Complex, Error, Float};

fn load() -> Result<Vec<f32>> {
    let mut file = File::open("burst.f32")?;

    let mut v = Vec::new();
    let mut buffer = [0u8; 4];
    while let Ok(bytes_read) = file.read(&mut buffer) {
        if bytes_read == 0 {
            break;
        }
        let float_value = f32::from_le_bytes(buffer);
        v.push(float_value);
    }
    Ok(v)
}

fn find_best_bin(data: &[Complex]) -> Option<usize> {
    // Never select the first two buckets.
    let skip = 2;

    // Convert to magnitude.
    let mag = data.iter().map(|x| x.norm_sqr().sqrt()).collect::<Vec<_>>();

    // We want a value above 80% of max.
    let thresh = mag
        .iter()
        .take(data.len())
        .skip(skip)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap()
        * 0.8;

    // Pick the first value that's above 80% of max and not still heading upwards.
    for (n, (v, nxt)) in mag.iter().zip(mag.iter().skip(1)).enumerate().skip(skip) {
        if *v > thresh && *v > *nxt {
            return Some(n);
        }
    }
    None
}

fn wpcr(samples: &[f32]) -> Option<Vec<u8>> {
    if samples.len() < 4 {
        return None;
    }

    let mid = 0.0;
    let sliced = samples.iter().map(|v| if *v > mid { 1.0 } else { 0.0 });
    let sliced_delayed = sliced.clone().skip(1);
    let mut d = sliced
        .zip(sliced_delayed)
        .map(|(a, b)| {
            let x = a - b;
            Complex::new(x * x, 0.0)
        })
        .collect::<Vec<_>>();

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(d.len());

    fft.process(&mut d);
    d.truncate(d.len() / 2);

    let bin = match find_best_bin(&d) {
        Some(bin) => bin,
        None => {
            eprintln!("No best bin");
            return None;
        }
    };
    let cycles_per_sample = bin as Float / samples.len() as Float;
    let mut clock_phase = {
        let t = 0.5 + d[bin].arg() / (std::f64::consts::PI * 2.0) as Float;
        if t > 0.5 {
            t
        } else {
            t + 1.0
        }
    };
    let samp_rate = 50000.0;
    let frequency = cycles_per_sample * samp_rate;
    let mut syms = Vec::with_capacity((samples.len() as Float / cycles_per_sample) as usize + 10);
    for i in 0..samples.len() {
        if clock_phase >= 1.0 {
            clock_phase -= 1.0;
            syms.push(if samples[i] > 0.0 { 1 } else { 0 });
        }
        clock_phase += cycles_per_sample;
    }
    eprintln!("Frequency: {} Hz", frequency);
    eprintln!("Phase: {} rad", d[bin].arg());
    Some(syms)
}

fn main() -> Result<()> {
    let samples = load()?;
    for d in wpcr(&samples).ok_or(Error::new("bleh"))? {
        println!("{d}");
    }
    Ok(())
}
