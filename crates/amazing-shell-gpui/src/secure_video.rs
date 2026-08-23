use std::io::{ErrorKind, Read};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

pub struct SecureVideoFrame {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub timestamp: u32,
    pub rgba: Vec<u8>,
}

pub enum SecureVideoEvent {
    Frame(SecureVideoFrame),
    Complete,
    Error(String),
}

pub fn spawn_secure_video_source() -> Result<Receiver<SecureVideoEvent>, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let driver = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .ok_or_else(|| "Amazing shell executable has no parent directory".to_string())?
        .join("amazing.exe");
    if !driver.is_file() {
        return Err(format!(
            "Amazing secure-video driver is missing: {}",
            driver.display()
        ));
    }
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("amazing-secure-video".into())
        .spawn(move || {
            let result = (|| -> Result<(), String> {
                let mut child = Command::new(driver)
                    .args(["secure-video-stream", &address.to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| format!("launch secure-video driver: {error}"))?;
                listener
                    .set_nonblocking(true)
                    .map_err(|error| error.to_string())?;
                let deadline = Instant::now() + Duration::from_secs(10);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error)
                            if error.kind() == ErrorKind::WouldBlock
                                || error.raw_os_error() == Some(10_035) => {}
                        Err(error) => return Err(error.to_string()),
                    }
                    if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                        return Err(format!(
                            "secure-video driver exited before connect: {status}"
                        ));
                    }
                    if Instant::now() >= deadline {
                        return Err("secure-video driver connection timed out".into());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                stream
                    .set_nonblocking(false)
                    .map_err(|error| error.to_string())?;
                stream
                    .set_read_timeout(Some(Duration::from_secs(15)))
                    .map_err(|error| error.to_string())?;
                read_stream(&mut stream, &sender)?;
                let status = child.wait().map_err(|error| error.to_string())?;
                if !status.success() {
                    return Err(format!("secure-video driver exited {status}"));
                }
                Ok(())
            })();
            match result {
                Ok(()) => {
                    let _ = sender.send(SecureVideoEvent::Complete);
                }
                Err(error) => {
                    let _ = sender.send(SecureVideoEvent::Error(error));
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(receiver)
}

fn read_stream(stream: &mut TcpStream, sender: &Sender<SecureVideoEvent>) -> Result<(), String> {
    let mut expected_index = 0u32;
    loop {
        let mut magic = [0u8; 4];
        match stream.read(&mut magic) {
            Ok(0) => return Ok(()),
            Ok(4) => {}
            Ok(read) => {
                stream
                    .read_exact(&mut magic[read..])
                    .map_err(|error| format!("truncated secure frame magic: {error}"))?;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        }
        if &magic != b"AMVF" {
            return Err("secure frame magic is invalid".into());
        }
        let index = read_u32(stream)?;
        let width = read_u32(stream)?;
        let height = read_u32(stream)?;
        let timestamp = read_u32(stream)?;
        let length = read_u32(stream)? as usize;
        let expected_length = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "secure frame dimensions overflow".to_string())?;
        if index != expected_index || length != expected_length || length > MAX_FRAME_BYTES {
            return Err("secure frame header is inconsistent".into());
        }
        let mut rgba = vec![0; length];
        stream
            .read_exact(&mut rgba)
            .map_err(|error| error.to_string())?;
        if !rgba.chunks_exact(4).all(|pixel| pixel[3] == 255) {
            return Err("secure frame alpha is not opaque".into());
        }
        sender
            .send(SecureVideoEvent::Frame(SecureVideoFrame {
                index,
                width,
                height,
                timestamp,
                rgba,
            }))
            .map_err(|_| "secure frame receiver closed".to_string())?;
        expected_index = expected_index.saturating_add(1);
    }
}

fn read_u32(stream: &mut impl Read) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    stream
        .read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn frame_header_reader_decodes_bounded_rgba() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(b"AMVF").unwrap();
            for value in [0u32, 2, 1, 90_000, 8] {
                stream.write_all(&value.to_le_bytes()).unwrap();
            }
            stream.write_all(&[1, 2, 3, 255, 4, 5, 6, 255]).unwrap();
        });
        let (sender, receiver) = mpsc::channel();
        let (mut stream, _) = listener.accept().unwrap();
        read_stream(&mut stream, &sender).unwrap();
        writer.join().unwrap();
        let SecureVideoEvent::Frame(frame) = receiver.recv().unwrap() else {
            panic!("expected frame");
        };
        assert_eq!((frame.index, frame.width, frame.height), (0, 2, 1));
        assert_eq!(frame.timestamp, 90_000);
        assert_eq!(frame.rgba.len(), 8);
    }
}
