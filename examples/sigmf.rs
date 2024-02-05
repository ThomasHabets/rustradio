use anyhow::Result;
use rustradio::sigmf::parse_meta;

fn main() -> Result<()> {
    let meta = parse_meta()?;
    println!("{:?}", meta);
    Ok(())
}
