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
fn rtp_jitter_probe_reorders_and_decodes_two_codecs() {
    let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("rtp-jitter-probe")
        .output()
        .expect("run jitter/codec probe");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("packets=3 reordered=yes"), "{stdout}");
    assert!(stdout.contains("duplicate=rejected"), "{stdout}");
    assert!(stdout.contains("codecs=L16+PCMU samples=3+2"), "{stdout}");
    assert!(stdout.contains("jitter/codec probe ok"), "{stdout}");
}

#[test]
fn opus_probe_encodes_decodes_and_conceals() {
    let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("opus-probe")
        .output()
        .expect("run Opus probe");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("decoded=960 samples PLC=960 samples"),
        "{stdout}"
    );
    assert!(stdout.contains("opus probe ok"), "{stdout}");
}

#[test]
fn video_frame_probe_reorders_and_drops_damage() {
    let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("video-frame-probe")
        .output()
        .expect("run video frame probe");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("packets=3 frame=16 bytes reordered=yes"),
        "{stdout}"
    );
    assert!(stdout.contains("damaged=dropped"), "{stdout}");
    assert!(stdout.contains("video frame probe ok"), "{stdout}");
}

#[test]
fn vp8_decode_probe_reconstructs_key_and_interframes() {
    let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("vp8-decode-probe")
        .output()
        .expect("run VP8 decode probe");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("interframes="), "{stdout}");
    assert!(!stdout.contains("interframes=0"), "{stdout}");
    assert!(stdout.contains("pixels_changed=yes"), "{stdout}");
    assert!(stdout.contains("vp8 decode probe ok"), "{stdout}");
}

#[test]
fn h264_roundtrip_probe_encodes_and_decodes_changed_pixels() {
    let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("h264-roundtrip-probe")
        .output()
        .expect("run H.264 roundtrip probe");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("size=32x32 pixels_changed=yes"), "{stdout}");
    assert!(stdout.contains("h264 roundtrip probe ok"), "{stdout}");
}

#[test]
fn sdp_roundtrip_command_reports_ice_dtls_media_summary() {
    let offer =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/phase9-offer.sdp");
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
    assert!(
        stdout.contains("SDP audio+video bundle=2 payloads=3"),
        "{stdout}"
    );
    assert!(stdout.contains("candidates=2"), "{stdout}");
    assert!(stdout.contains("rtpmaps=3"), "{stdout}");
    assert!(stdout.contains("ICE=ferric"), "{stdout}");
    assert!(stdout.contains("setup=actpass"), "{stdout}");
    assert!(stdout.contains("roundtrip ok"), "{stdout}");
}

#[test]
fn secure_media_loopback_uses_vetted_dtls_srtp() {
    let result = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("secure-media-loopback")
        .output()
        .expect("run secure-media loopback artifact");
    assert!(
        result.status.success(),
        "secure-media loopback failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).expect("UTF-8 command output");
    assert!(
        stdout.contains("ICE state=Completed credentials=SDP"),
        "{stdout}"
    );
    assert!(stdout.contains("UDP transport=127.0.0.1:"), "{stdout}");
    assert!(
        stdout.contains("SDP identity=ok roles=active/passive"),
        "{stdout}"
    );
    assert!(stdout.contains("DTLS1.2"), "{stdout}");
    assert!(
        stdout.contains("TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256"),
        "{stdout}"
    );
    assert!(stdout.contains("SRTP profile=AEAD_AES_128_GCM"), "{stdout}");
    assert!(stdout.contains("fingerprints=ok"), "{stdout}");
    assert!(stdout.contains("SRTP payload=15 bytes"), "{stdout}");
    assert!(stdout.contains("SRTCP packets=1"), "{stdout}");
    assert!(
        stdout.contains("SRTP tamper=rejected replay=rejected"),
        "{stdout}"
    );
    assert!(
        stdout.contains("SRTCP tamper=rejected replay=rejected"),
        "{stdout}"
    );
    assert!(stdout.contains("secure loopback ok"), "{stdout}");
}

#[test]
fn secure_media_two_processes_exchange_protected_media() {
    let result = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .arg("secure-media-processes")
        .output()
        .expect("run two-process secure-media artifact");
    assert!(
        result.status.success(),
        "two-process secure media failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).expect("UTF-8 command output");
    assert!(stdout.contains("processes=2"), "{stdout}");
    assert!(stdout.contains("signaling=tcp-loopback"), "{stdout}");
    assert!(stdout.contains("ICE credentials=SDP"), "{stdout}");
    assert!(stdout.contains("codecs=SDP Opus+L16+PCMU+H264"), "{stdout}");
    assert!(
        stdout.contains("ICE live-loss=2 sends=3 487=resolved prflx=learned"),
        "{stdout}"
    );
    assert!(stdout.contains("video=H264 secure 32x32"), "{stdout}");
    assert!(stdout.contains("consent=Fresh expiry=detected"), "{stdout}");
    assert!(stdout.contains("RTP parent->child=ok"), "{stdout}");
    assert!(stdout.contains("RTP child->parent=ok"), "{stdout}");
    assert!(
        stdout.contains(
            "playout=adaptive loss=1 conceal=160 Opus parent->child=960 PCMU child->parent=320"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("SRTCP SR+CNAME child->parent=ok"),
        "{stdout}"
    );
    assert!(
        stdout.contains("RTCP RR+CNAME parent->child=ok"),
        "{stdout}"
    );
    assert!(stdout.contains("RTCP stats=computed"), "{stdout}");
    assert!(stdout.contains("two-process interop ok"), "{stdout}");
}
