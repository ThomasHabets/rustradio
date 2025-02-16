use std::path::Path;
use std::process::Command;

use anyhow::Result;
use tempfile::tempdir;

#[test]
#[ignore]
fn e2e_test_wa8lmf_intact() -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::{BufReader, Read};

    for (filename, want) in [
        (
            "wa8lmf-cd-track1.au",
            "dcd04965aa898d9c12ab268422423d648e166c46f0ab4b5b9b8b3ebc4476d588",
        ),
        (
            "aprs-9600-50k.c32",
            "191c7c2aa36bd487db0bd34945eee89108b3fa1373c1b4b2592e741002cd9b3d",
        ),
    ] {
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
        assert_eq!(format!("{:x}", hash), want);
    }
    Ok(())
}

fn count_files_in_dir(dir_path: &Path) -> usize {
    match std::fs::read_dir(dir_path) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .map(|e| {
                println!("File: {e:?}");
                e
            })
            .count(),
        Err(err) => {
            eprintln!("Failed to read directory: {}", err);
            0
        }
    }
}

#[test]
#[ignore]
fn e2e_ax25_decoding() {
    for (example, filename, sample_rate, opts, want) in [
        (
            "ax25-1200-rx",
            "wa8lmf-cd-track1.au",
            44100,
            vec!["-a"],
            909,
        ),
        ("ax25-9600-wpcr", "aprs-9600-50k.c32", 50000, vec![], 1),
    ] {
        let temp_dir = tempdir().unwrap();
        let testfile = Path::new("tests/testdata").join(filename);
        let mut args: Vec<_> = ["run", "--release", "--example", example, "--"]
            .iter()
            .map(|x| x.to_string())
            .collect();
        args.extend(
            [
                "-r",
                &format!("{}", testfile.as_path().display()),
                "-o",
                &format!("{}", temp_dir.path().display()),
                "--sample_rate",
                &sample_rate.to_string(),
            ]
            .iter()
            .map(|x| x.to_string()),
        );
        args.extend(opts.iter().map(|x| x.to_string()));
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
        let decodes = count_files_in_dir(temp_dir.path());
        assert_eq!(decodes, want);
    }
}
