//! Persistent rigctld PTT connection using its default (non-VFO) protocol.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use anyhow::{Context, Result, ensure};

pub(super) struct Rigctld {
    stream: BufReader<TcpStream>,
    // Also true when key-on was sent but its acknowledgement was lost.
    keyed: bool,
}

impl Rigctld {
    pub(super) fn connect(address: &str) -> Result<Self> {
        let timeout = Duration::from_secs(2);
        let mut last_error = None;
        let stream = address
            .to_socket_addrs()
            .with_context(|| format!("invalid rigctld address {address:?}"))?
            .find_map(
                |address| match TcpStream::connect_timeout(&address, timeout) {
                    Ok(stream) => Some(stream),
                    Err(e) => {
                        last_error = Some(e);
                        None
                    }
                },
            )
            .with_context(|| {
                format!("could not connect to rigctld at {address}: {last_error:?}")
            })?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        stream.set_nodelay(true)?;
        let mut rig = Self {
            stream: BufReader::new(stream),
            keyed: false,
        };
        rig.set_ptt(false)
            .context("initial rigctld PTT-off command failed")?;
        Ok(rig)
    }

    pub(super) fn set_ptt(&mut self, on: bool) -> Result<()> {
        self.keyed |= on;
        self.stream
            .get_mut()
            .write_all(if on { b"T 1\n" } else { b"T 0\n" })
            .context("writing rigctld PTT command")?;
        let mut response = String::new();
        (&mut self.stream)
            .take(256)
            .read_line(&mut response)
            .context("reading rigctld PTT acknowledgement")?;
        ensure!(
            response.ends_with('\n') && response.trim_end() == "RPRT 0",
            "rigctld PTT {} failed: expected RPRT 0, received {:?}",
            if on { "on" } else { "off" },
            response
        );
        self.keyed = on;
        Ok(())
    }
}

impl Drop for Rigctld {
    fn drop(&mut self) {
        if self.keyed
            && let Err(e) = self.set_ptt(false)
        {
            log::error!("Failed to release rigctld PTT: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn server(replies: Vec<(&'static str, &'static str)>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let thread = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut stream = BufReader::new(stream);
            for (command, reply) in replies {
                let mut line = String::new();
                stream.read_line(&mut line).unwrap();
                assert_eq!(line, command);
                stream.get_mut().write_all(reply.as_bytes()).unwrap();
            }
        });
        (address, thread)
    }

    #[test]
    fn keys_and_releases_on_drop() -> Result<()> {
        let (address, thread) = server(vec![
            ("T 0\n", "RPRT 0\n"),
            ("T 1\n", "RPRT 0\n"),
            ("T 0\n", "RPRT 0\n"),
        ]);
        let mut rig = Rigctld::connect(&address)?;
        rig.set_ptt(true)?;
        drop(rig);
        thread.join().unwrap();
        Ok(())
    }

    #[test]
    fn rejects_failed_or_malformed_replies_and_still_releases() -> Result<()> {
        for reply in ["RPRT -1\n", "unexpected\n"] {
            let (address, thread) = server(vec![
                ("T 0\n", "RPRT 0\n"),
                ("T 1\n", reply),
                ("T 0\n", "RPRT 0\n"),
            ]);
            let mut rig = Rigctld::connect(&address)?;
            let error = rig.set_ptt(true).unwrap_err().to_string();
            assert!(error.contains("expected RPRT 0"));
            drop(rig);
            thread.join().unwrap();
        }
        Ok(())
    }

    #[test]
    fn connection_failure_is_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        drop(listener);
        assert!(
            Rigctld::connect(&address)
                .err()
                .unwrap()
                .to_string()
                .contains("could not connect")
        );
    }

    #[test]
    fn truncated_acknowledgement_is_rejected() -> Result<()> {
        let (address, thread) = server(vec![("T 0\n", "RPRT 0")]);
        assert!(Rigctld::connect(&address).is_err());
        thread.join().unwrap();
        Ok(())
    }

    #[test]
    fn lost_key_on_acknowledgement_still_attempts_key_off() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        let thread = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).unwrap();
            assert_eq!(line, "T 0\n");
            stream.get_mut().write_all(b"RPRT 0\n").unwrap();
            line.clear();
            stream.read_line(&mut line).unwrap();
            assert_eq!(line, "T 1\n");
            stream
                .get_ref()
                .shutdown(std::net::Shutdown::Write)
                .unwrap();
            line.clear();
            stream.read_line(&mut line).unwrap();
            assert_eq!(line, "T 0\n");
        });
        let mut rig = Rigctld::connect(&address)?;
        assert!(rig.set_ptt(true).is_err());
        drop(rig);
        thread.join().unwrap();
        Ok(())
    }
}
