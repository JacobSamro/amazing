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
