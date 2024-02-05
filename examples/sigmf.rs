use anyhow::Result;
use rustradio::sigmf::parse_meta;

fn main() -> Result<()> {
    let meta = parse_meta("data/1876954_7680KSPS_srsRAN_Project_gnb_short.sigmf")?;
    println!("{:?}", meta);
    rustradio::sigmf::write("blah.js", 50000.0, 144800000.0)?;
    Ok(())
}
