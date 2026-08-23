use std::process::Command;

#[test]
fn webrtc_loopback_command_completes_stun_and_rtp_exchange() {
    let result = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("webrtc-loopback")
        .output()
        .expect("run WebRTC loopback artifact");
    assert!(
        result.status.success(),
        "loopback failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).expect("UTF-8 command output");
    assert!(stdout.contains("STUN mapped 127.0.0.1:"), "{stdout}");
    assert!(stdout.contains("RTP seq=7"), "{stdout}");
    assert!(stdout.contains("payload=15 bytes"), "{stdout}");
    assert!(stdout.contains("loopback ok"), "{stdout}");
}
