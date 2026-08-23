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
    assert!(stdout.contains("ICE state=Completed"), "{stdout}");
    assert!(stdout.contains("integrity=ok"), "{stdout}");
    assert!(stdout.contains("RTCP packets=1 octets=15"), "{stdout}");
    assert!(stdout.contains("loopback ok"), "{stdout}");
}

#[test]
fn sdp_roundtrip_command_reports_ice_dtls_media_summary() {
    let offer = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/phase9-offer.sdp");
    let result = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .args(["sdp-roundtrip", offer.to_str().expect("UTF-8 offer path")])
        .output()
        .expect("run SDP artifact");
    assert!(
        result.status.success(),
        "SDP artifact failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).expect("UTF-8 command output");
    assert!(stdout.contains("SDP audio"), "{stdout}");
    assert!(stdout.contains("candidates=1"), "{stdout}");
    assert!(stdout.contains("rtpmaps=1"), "{stdout}");
    assert!(stdout.contains("ICE=ferric"), "{stdout}");
    assert!(stdout.contains("setup=actpass"), "{stdout}");
    assert!(stdout.contains("roundtrip ok"), "{stdout}");
}
