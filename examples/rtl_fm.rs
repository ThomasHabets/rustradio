// Not even remotely working yet.

use anyhow::Result;

#[cfg(feature = "rtlsdr")]
fn main() -> Result<()> {
    println!("Hello world");
    println!("Device count: {}", rtlsdr::get_device_count());
    println!("Device name: {}", rtlsdr::get_device_name(0));
    let ss = rtlsdr::get_device_usb_strings(0).unwrap();
    println!(
        "Manufacturer: {} Product: {} Serial: {}",
        ss.manufacturer, ss.product, ss.serial
    );

    let mut dev = rtlsdr::open(0).unwrap();
    println!("Tuner type: {:?}", dev.get_tuner_type());
    // dev.set_direct_sampling
    dev.set_center_freq(868_000_000).unwrap();
    println!("Allowed tuner gains: {:?}", dev.get_tuner_gains().unwrap());
    dev.set_tuner_gain(27).unwrap();
    println!("Tuner gain: {}", dev.get_tuner_gain());
    let (xtal_clock_freq, xtal_tuner_freq) = dev.get_xtal_freq().unwrap();
    println!("XTAL: {xtal_clock_freq} {xtal_tuner_freq}");
    // dev.set_tuner_if_gain(…);
    // dev.set_tuner_gain_mode
    // dev.set_agc_mode
    dev.set_sample_rate(1_024_000).unwrap();
    //dev.
    dev.reset_buffer().unwrap();
    dev.read_sync(8192).expect("read to work");
    println!("Done");
    Ok(())
}

#[cfg(not(feature = "rtlsdr"))]
fn main() -> Result<()> {
    use rustradio::Error;
    Err(Error::new("RTL SDR feature not enabled").into())
}
