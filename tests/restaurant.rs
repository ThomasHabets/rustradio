use std::path::Path;
use std::process::Command;

use anyhow::Result;

#[test]
#[ignore]
fn e2e_test_kungshallen() -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::{BufReader, Read};

    #[allow(clippy::single_element_loop)]
    for (filename, want) in [(
        "kungshallen-125k.c32",
        "fdbd14bc8d8ead0a5c8a55c2c84cb14fc3890974fdc8d0e80e3bb686723791e8",
    )] {
        let file = File::open(Path::new("tests/testdata").join(filename))?;
        let mut reader = BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];

        loop {
            let rc = reader.read(&mut buffer)?;
            if rc == 0 {
                break;
            }
            hasher.update(&buffer[..rc]);
        }
        let hash = hasher.finalize();
        assert_eq!(format!("{hash:x}"), want);
    }
    Ok(())
}

#[test]
#[ignore]
fn e2e_restaurant_decoding() -> Result<()> {
    #[allow(clippy::single_element_loop)]
    for (example, filename, sample_rate, want) in [(
        "restaurant_pager",
        "kungshallen-125k.c32",
        125_000,
        "Restaurant-Pager: id=0xf9bf pager=11 function=Buzz (0xd) repeats=39 raw=0x1f37f7b time=0.791144s",
    )] {
        let testfile = Path::new("tests/testdata").join(filename);
        let mut args: Vec<_> = [
            "run",
            "--release",
            "--example",
            example,
            "--",
            "--sample-rate",
            &sample_rate.to_string(),
        ]
        .iter()
        .map(|x| x.to_string())
        .collect();
        args.extend(
            ["file", &format!("{}", testfile.as_path().display())]
                .iter()
                .map(|x| x.to_string()),
        );
        eprintln!("Running test {example} with: {args:?}");
        let output = Command::new("cargo")
            .args(&args)
            .output()
            .expect("Failed to execute example binary");
        assert!(
            output.status.success(),
            "Binary did not run successfully: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let out = String::from_utf8(output.stdout)?;
        assert!(out.contains(want), "Output did not include {want}: {out}");
    }
    Ok(())
}
