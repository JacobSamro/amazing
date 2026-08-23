use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "amazing-phase9-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create phase9 temp dir");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pcm16_wav(sample_rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let block_align = channels * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[test]
fn render_command_loads_and_paints_relative_image_media() {
    let temp = TempDir::new();
    let page = temp.0.join("page.html");
    let image = temp.0.join("pixel.ppm");
    let output = temp.0.join("render.bmp");
    fs::write(
        &page,
        b"<body style=\"margin:0\"><img src=\"pixel.ppm\"></body>",
    )
    .expect("write page");
    fs::write(&image, b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00")
        .expect("write PPM");

    let result = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .args([
            "render",
            page.to_str().expect("UTF-8 page path"),
            output.to_str().expect("UTF-8 output path"),
            "2",
        ])
        .output()
        .expect("run amazing render");
    assert!(
        result.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let decoded = amazing_media::decode_image(&fs::read(output).expect("read output BMP"))
        .expect("decode rendered BMP");
    assert_eq!(decoded.width, 2);
    let painted = decoded.pixels.chunks_exact(decoded.width as usize * 4).any(|row| {
        row == [255, 0, 0, 255, 0, 255, 0, 255]
    });
    assert!(painted, "decoded image pixels were not present in the rendered page");
}

#[test]
fn audio_info_command_reports_decoded_pcm_stream() {
    let temp = TempDir::new();
    let wav = temp.0.join("tone.wav");
    fs::write(&wav, pcm16_wav(8_000, 1, &[i16::MIN, 0, i16::MAX]))
        .expect("write WAV");

    let result = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .args(["audio-info", wav.to_str().expect("UTF-8 WAV path")])
        .output()
        .expect("run amazing audio-info");
    assert!(
        result.status.success(),
        "audio-info failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).expect("UTF-8 command output");
    assert!(stdout.contains("8000 Hz"), "{stdout}");
    assert!(stdout.contains("1 channel"), "{stdout}");
    assert!(stdout.contains("3 frames"), "{stdout}");
    assert!(stdout.contains("0.000375 s"), "{stdout}");
    assert!(stdout.contains("peak 32768"), "{stdout}");
}
