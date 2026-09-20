//! WebRTC signaling, codec, and secure transport probes.
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, ExitCode, ExitStatus};
use std::time::{Duration, Instant};
use crate::browser_peer;
pub(crate) fn cmd_turn_probe() -> ExitCode {
    match amazing_webrtc::run_turn_udp_probe() {
        Ok(report) => {
            println!(
                "TURN challenge={} relay={} mapped={} lifetime={}s permission={} data={}/{} nominate={} delete={}; turn probe ok",
                report.challenge_code,
                report.relayed_address,
                report.mapped_address,
                report.lifetime_seconds,
                if report.permission_created {
                    "ok"
                } else {
                    "missing"
                },
                report.client_to_peer_bytes,
                report.peer_to_client_bytes,
                if report.relay_nominated {
                    "ok"
                } else {
                    "missing"
                },
                if report.allocation_deleted {
                    "ok"
                } else {
                    "missing"
                },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing turn-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_rtp_jitter_probe() -> ExitCode {
    match amazing_webrtc::run_jitter_codec_probe() {
        Ok(report) => {
            println!(
                "RTP jitter packets={} reordered={} duplicate={} codecs=L16+PCMU samples={}+{}; jitter/codec probe ok",
                report.emitted_packets,
                if report.reordered { "yes" } else { "no" },
                if report.duplicate_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
                report.l16_samples,
                report.pcmu_samples,
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing rtp-jitter-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_opus_probe() -> ExitCode {
    match amazing_webrtc_codecs::run_opus_probe() {
        Ok(report) => {
            println!(
                "Opus encoded={} bytes decoded={} samples PLC={} samples; opus probe ok",
                report.encoded_bytes, report.decoded_samples, report.concealed_samples
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing opus-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_video_frame_probe() -> ExitCode {
    match amazing_webrtc::run_video_frame_probe() {
        Ok(report) => {
            println!(
                "video RTP packets={} frame={} bytes reordered={} damaged={}; video frame probe ok",
                report.packets,
                report.frame_bytes,
                if report.reordered { "yes" } else { "no" },
                if report.damaged_dropped {
                    "dropped"
                } else {
                    "accepted"
                },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing video-frame-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_vp8_decode_probe() -> ExitCode {
    match amazing_webrtc_codecs::run_vp8_probe(include_bytes!(
        "../../../../examples/vp80-02-inter-1402.ivf"
    )) {
        Ok(report) => {
            println!(
                "VP8 frames={} interframes={} size={}x{} pixels_changed={}; vp8 decode probe ok",
                report.frames,
                report.interframes,
                report.width,
                report.height,
                if report.checksum_changed { "yes" } else { "no" },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing vp8-decode-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_h264_roundtrip_probe() -> ExitCode {
    match amazing_webrtc_codecs::run_h264_probe() {
        Ok(report) => {
            println!(
                "H264 first={} bytes delta={} bytes size={}x{} pixels_changed={}; h264 roundtrip probe ok",
                report.first_bytes,
                report.second_bytes,
                report.width,
                report.height,
                if report.pixels_changed { "yes" } else { "no" },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing h264-roundtrip-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_h264_rtp_probe() -> ExitCode {
    match amazing_webrtc_codecs::run_h264_rtp_probe() {
        Ok(report) => {
            println!(
                "H264 AnnexB={} bytes RTP={} packets FU-A={} max-payload={} size={}x{}; h264 rtp probe ok",
                report.annex_b_bytes,
                report.rtp_packets,
                report.fu_a_packets,
                report.max_payload_bytes,
                report.width,
                report.height,
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing h264-rtp-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 9.9 artifact: vetted DTLS 1.2 key agreement and SRTP/SRTCP proof.
pub(crate) fn cmd_secure_media_loopback() -> ExitCode {
    match amazing_webrtc_crypto::run_secure_loopback() {
        Ok(report) => {
            println!(
                "ICE state={} credentials={}; UDP transport={} -> {}; SDP identity={} roles={}/{}; {} {}; SRTP profile={} keying={} bytes; fingerprints={}; SRTP payload={} bytes protected={} bytes; SRTCP packets={} protected={} bytes; SRTP tamper={} replay={}; SRTCP tamper={} replay={}; secure loopback ok",
                report.ice_state,
                if report.ice_credentials_bound {
                    "SDP"
                } else {
                    "local"
                },
                report.udp_client_address,
                report.udp_server_address,
                if report.sdp_identity_verified {
                    "ok"
                } else {
                    "failed"
                },
                report.client_setup,
                report.server_setup,
                report.dtls_version,
                report.cipher_suite,
                report.srtp_profile,
                report.exported_keying_material_bytes,
                if report.fingerprints_verified {
                    "ok"
                } else {
                    "failed"
                },
                report.rtp_payload_bytes,
                report.protected_rtp_bytes,
                report.rtcp_packet_count,
                report.protected_rtcp_bytes,
                if report.srtp_tamper_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
                if report.srtp_replay_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
                if report.srtcp_tamper_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
                if report.srtcp_replay_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing secure-media-loopback: {error}");
            ExitCode::FAILURE
        }
    }
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child guard is active")
    }

    fn finish(mut self, timeout: Duration) -> Result<ExitStatus, String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child_mut().try_wait() {
                Ok(Some(status)) => {
                    self.child.take();
                    return Ok(status);
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let child = self.child_mut();
                    let _ = child.kill();
                    let reap = child.wait();
                    return Err(match reap {
                        Ok(_) => {
                            self.child.take();
                            "secure peer completion timed out; child killed and reaped".into()
                        }
                        Err(error) => format!(
                            "secure peer completion timed out; reap after kill failed and guard will retry: {error}"
                        ),
                    });
                }
                Err(wait_error) => {
                    let child = self.child_mut();
                    let _ = child.kill();
                    let reap = child.wait();
                    return Err(match reap {
                        Ok(_) => {
                            self.child.take();
                            format!("poll secure peer: {wait_error}; child killed and reaped")
                        }
                        Err(reap_error) => format!(
                            "poll secure peer: {wait_error}; reap after kill failed and guard will retry: {reap_error}"
                        ),
                    });
                }
            }
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub(crate) fn cmd_secure_media_processes() -> ExitCode {
    match run_secure_media_processes(None) {
        Ok(()) => {
            println!(
                "processes=2 signaling=tcp-loopback; ICE credentials=SDP; codecs=SDP Opus+L16+PCMU+H264; ICE live-loss=2 sends=3 487=resolved prflx=learned; consent=Fresh expiry=detected; RTP parent->child=ok; RTP child->parent=ok; playout=adaptive loss=1 conceal=160 Opus parent->child=960 PCMU child->parent=320; video=H264 secure frames=6 32x32 clock=90k changed=yes loss=RTP1 recovery=PLI+IDR; avsync=RTCP-SR 6/6 aligned; SRTCP SR+CNAME child->parent=ok; RTCP RR+CNAME parent->child=ok; RTCP stats=computed; two-process interop ok"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing secure-media-processes: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_secure_video_stream(frame_sink: Option<&str>) -> ExitCode {
    let Some(frame_sink) = frame_sink else {
        eprintln!("usage: amazing secure-video-stream <host:port>");
        return ExitCode::FAILURE;
    };
    match run_secure_media_processes(Some(frame_sink)) {
        Ok(()) => {
            println!(
                "secure AV stream audio=PCM48k/30s frames=6 size=32x32 timestamps=90k pace=RTCP decode=incremental loss=RTP1 recovery=PLI+IDR; stream ok"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing secure-video-stream: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_secure_media_peer(control_address: Option<&str>, frame_sink: Option<&str>) -> ExitCode {
    match run_secure_media_peer(control_address, frame_sink) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amazing secure-media-peer: {error}");
            ExitCode::FAILURE
        }
    }
}

fn verify_ice_robustness_probe() -> Result<(), String> {
    let replacement_tiebreaker =
        amazing_webrtc_crypto::secure_random_ice_tiebreaker().map_err(|error| error.to_string())?;
    let report = amazing_webrtc::run_live_ice_robustness_probe(replacement_tiebreaker)
        .map_err(|error| error.to_string())?;
    if report.request_sends != 3
        || report.injected_drops != 2
        || !report.role_switched
        || !report.peer_reflexive_inserted
    {
        return Err("live ICE robustness probe failed".into());
    }
    Ok(())
}

fn negotiated_audio_decoder(
    codecs: &amazing_webrtc::NegotiatedAudioCodecs,
    payload_type: u8,
) -> Result<Box<dyn amazing_webrtc::RtpAudioDecoder>, String> {
    let codec = codecs
        .codec(payload_type)
        .map_err(|error| error.to_string())?;
    if codec.encoding.eq_ignore_ascii_case("OPUS") {
        Ok(Box::new(
            amazing_webrtc_codecs::OpusDecoderAdapter::from_negotiated(codec)
                .map_err(|error| error.to_string())?,
        ))
    } else {
        codecs
            .decoder(payload_type)
            .map_err(|error| error.to_string())
    }
}

struct NegotiatedVideoMedia<'a> {
    section: &'a amazing_webrtc::SdpMediaSection,
    payload_type: u8,
    clock_rate: u32,
}

fn negotiated_video_media<'a>(
    offer: &'a amazing_webrtc::SdpDescription,
    answer: &amazing_webrtc::SdpDescription,
    encoding: &str,
) -> Result<NegotiatedVideoMedia<'a>, String> {
    let offered = offer
        .media("video")
        .ok_or_else(|| "offer has no video media".to_string())?;
    let accepted = answer
        .media("video")
        .ok_or_else(|| "answer has no video media".to_string())?;
    let offered_map = offered
        .rtp_maps
        .iter()
        .find(|mapping| {
            mapping.encoding.eq_ignore_ascii_case(encoding)
                && accepted
                    .rtp_maps
                    .iter()
                    .any(|accepted| accepted == *mapping)
        })
        .ok_or_else(|| format!("offer and answer have no common {encoding} video codec"))?;
    if offered.media != "video"
        || accepted.media != "video"
        || offered.mid != accepted.mid
        || offered_map.clock_rate != 90_000
    {
        return Err("video offer/answer mapping changed".into());
    }
    Ok(NegotiatedVideoMedia {
        section: offered,
        payload_type: offered_map.payload_type,
        clock_rate: offered_map.clock_rate,
    })
}

fn run_secure_media_processes(frame_sink: Option<&str>) -> Result<(), String> {
    verify_ice_robustness_probe()?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("bind signaling listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure signaling listener: {error}"))?;
    let signaling_address = listener.local_addr().map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut child_command = Command::new(executable);
    child_command
        .arg("__secure-media-peer")
        .arg(signaling_address.to_string());
    if let Some(frame_sink) = frame_sink {
        child_command.arg(frame_sink);
    }
    let child = child_command
        .spawn()
        .map_err(|error| format!("spawn secure peer: {error}"))?;
    let mut child = ChildGuard::new(child);
    let mut control = accept_control_connection(&listener, &mut child, Duration::from_secs(5))?;
    configure_control_stream(&control)?;
    let mut control_reader =
        BufReader::new(control.try_clone().map_err(|error| error.to_string())?);
    let offer = read_signaled_description(&mut control_reader, "OFFER")?;

    let socket = std::net::UdpSocket::bind("127.0.0.1:0")
        .map_err(|error| format!("bind parent UDP: {error}"))?;
    let parent_address = std_socket_v4(&socket)?;
    let child_address = description_candidate(&offer)?;
    socket
        .connect(child_address)
        .map_err(|error| format!("connect parent UDP: {error}"))?;
    let identity = amazing_webrtc_crypto::SecureIdentity::generate("ferric-parent")
        .map_err(|error| error.to_string())?;
    let answer = amazing_webrtc_crypto::build_local_description(
        parent_address,
        identity.fingerprint(),
        "active",
        "parenthost",
        "parent",
        "FerricParentPassword123456",
        400,
    )
    .map_err(|error| error.to_string())?;
    write_signaled_description(&mut control, "ANSWER", &answer)?;
    let negotiated_codecs = amazing_webrtc::negotiate_audio_codecs(&offer, &answer)
        .map_err(|error| error.to_string())?;
    let video_payload_type = negotiated_video_media(&offer, &answer, "H264")?.payload_type;

    let mut endpoint = amazing_webrtc_crypto::SecureUdpEndpoint::establish(
        amazing_webrtc_crypto::SecureEndpointConfig {
            socket,
            identity,
            local_description: answer,
            remote_description: offer,
            local_is_offerer: false,
            ice_role: amazing_webrtc_crypto::IceRole::Controlling,
        },
    )
    .map_err(|error| error.to_string())?;
    let opus_codec = negotiated_codecs
        .codec_by_encoding("opus")
        .map_err(|error| error.to_string())?;
    let mut opus_encoder = amazing_webrtc_codecs::OpusEncoderAdapter::from_negotiated(opus_codec)
        .map_err(|error| error.to_string())?;
    let opus_pcm: Vec<i16> = (0..960)
        .map(|index| (((index % 96) - 48) * 400) as i16)
        .collect();
    let outbound = opus_encoder
        .encode(41, 960, 0x5041_5245, &opus_pcm)
        .map_err(|error| error.to_string())?;
    let outbound_wire = amazing_webrtc::encode_rtp(&outbound).map_err(|error| error.to_string())?;
    endpoint
        .send_rtp(&outbound_wire)
        .map_err(|error| error.to_string())?;
    endpoint
        .wait_for_consent_due()
        .map_err(|error| error.to_string())?;
    if endpoint
        .refresh_consent()
        .map_err(|error| error.to_string())?
        != amazing_webrtc::ConsentState::Fresh
    {
        return Err("live consent refresh did not remain Fresh".into());
    }
    let expiry_probe =
        amazing_webrtc::ConsentFreshness::new(0, 5_000).map_err(|error| error.to_string())?;
    if expiry_probe.state(30_000) != amazing_webrtc::ConsentState::Expired {
        return Err("consent expiry probe did not expire at thirty seconds".into());
    }
    let child_rtp = endpoint.receive_rtp().map_err(|error| error.to_string())?;
    let child_rtp = amazing_webrtc::parse_rtp(&child_rtp).map_err(|error| error.to_string())?;
    let child_rtp_later = endpoint.receive_rtp().map_err(|error| error.to_string())?;
    let child_rtp_later =
        amazing_webrtc::parse_rtp(&child_rtp_later).map_err(|error| error.to_string())?;
    if child_rtp.payload_type != 0
        || child_rtp.sequence_number != 84
        || child_rtp_later.sequence_number != 86
        || child_rtp_later.ssrc != child_rtp.ssrc
    {
        return Err("child process returned unexpected RTP".into());
    }
    let child_config = negotiated_codecs
        .jitter_config(child_rtp.payload_type, child_rtp.ssrc, 60, 8)
        .map_err(|error| error.to_string())?;
    let mut child_jitter = amazing_webrtc::RtpJitterBuffer::new_adaptive(
        child_config,
        amazing_webrtc::AdaptivePlayoutConfig {
            max_delay_ms: 200,
            jitter_multiplier: 4,
        },
    )
    .map_err(|error| error.to_string())?;
    child_jitter
        .push(child_rtp.clone(), 0)
        .map_err(|error| error.to_string())?;
    child_jitter
        .push(child_rtp_later.clone(), 5)
        .map_err(|error| error.to_string())?;
    let first_playout = child_jitter.target_delay_ms();
    let Some(amazing_webrtc::JitterOutput::Packet(played_child_rtp)) =
        child_jitter.pop_ready(first_playout)
    else {
        return Err("child RTP did not reach jitter playout".into());
    };
    let mut child_decoder = negotiated_audio_decoder(&negotiated_codecs, child_rtp.payload_type)?;
    let child_audio = child_decoder
        .decode(&played_child_rtp)
        .map_err(|error| error.to_string())?;
    if child_audio.samples.len() != 160 {
        return Err("child audio playout sample count was incorrect".into());
    }
    let Some(amazing_webrtc::JitterOutput::Gap {
        estimated_timestamp: Some(missing_timestamp),
        duration_timestamp_units: Some(missing_samples),
        ..
    }) = child_jitter.pop_ready(first_playout + 40)
    else {
        return Err("child RTP loss did not reach jitter concealment".into());
    };
    let concealed = child_decoder
        .conceal(missing_timestamp, missing_samples as usize)
        .map_err(|error| error.to_string())?;
    if concealed.samples != vec![0; 160] {
        return Err("child PCMU concealment samples were incorrect".into());
    }
    let Some(amazing_webrtc::JitterOutput::Packet(played_child_later)) =
        child_jitter.pop_ready(first_playout + 40)
    else {
        return Err("child RTP after loss did not reach jitter playout".into());
    };
    let later_audio = child_decoder
        .decode(&played_child_later)
        .map_err(|error| error.to_string())?;
    if later_audio.samples != vec![32_124; 160] || child_jitter.stats().gaps != 1 {
        return Err("child PCMU post-loss samples were incorrect".into());
    }
    let mut reception_stats = amazing_webrtc::RtpReceptionStats::new(child_rtp.ssrc);
    reception_stats
        .observe(&child_rtp, child_rtp.timestamp)
        .map_err(|error| error.to_string())?;
    reception_stats
        .observe(&child_rtp_later, child_rtp_later.timestamp)
        .map_err(|error| error.to_string())?;
    let child_rtcp = endpoint.receive_rtcp().map_err(|error| error.to_string())?;
    let child_rtcp = amazing_webrtc::parse_rtcp_compound_sender_report(&child_rtcp)
        .map_err(|error| error.to_string())?;
    if child_rtcp.sender.ssrc != child_rtp.ssrc
        || child_rtcp.sender.rtp_timestamp != child_rtp_later.timestamp
        || child_rtcp.sender.packet_count != 2
        || child_rtcp.sender.octet_count != 320
        || child_rtcp.cname != "child@ferric"
    {
        return Err("child process returned inconsistent SRTCP".into());
    }
    read_control_marker(&mut control_reader, "READY_RR")?;
    let reception = reception_stats
        .report((child_rtcp.sender.ntp_timestamp >> 16) as u32, 0)
        .map_err(|error| error.to_string())?;
    let receiver_report = amazing_webrtc::RtcpCompoundReceiverReport {
        receiver_ssrc: 0x5252_4356,
        reception,
        cname: "parent@ferric".into(),
    };
    let receiver_report = amazing_webrtc::encode_rtcp_compound_receiver_report(&receiver_report)
        .map_err(|error| error.to_string())?;
    endpoint
        .send_rtcp(&receiver_report)
        .map_err(|error| format!("send compound receiver report: {error}"))?;

    const AV_SYNC_NTP: u64 = 0xe223_4567_0000_0000;
    let audio_sender = amazing_webrtc::RtcpCompoundSenderReport {
        sender: amazing_webrtc::RtcpSenderReport {
            ssrc: outbound.ssrc,
            ntp_timestamp: AV_SYNC_NTP,
            rtp_timestamp: outbound.timestamp,
            packet_count: 1,
            octet_count: outbound.payload.len() as u32,
        },
        cname: "parent-audio@amazing".into(),
    };
    let video_sender = amazing_webrtc::RtcpCompoundSenderReport {
        sender: amazing_webrtc::RtcpSenderReport {
            ssrc: 0x5649_4445,
            ntp_timestamp: AV_SYNC_NTP,
            rtp_timestamp: 90_000,
            packet_count: 0,
            octet_count: 0,
        },
        cname: "parent-video@amazing".into(),
    };
    for report in [&audio_sender, &video_sender] {
        endpoint
            .send_rtcp(
                &amazing_webrtc::encode_rtcp_compound_sender_report(report)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    let mut h264_encoder =
        amazing_webrtc_codecs::H264EncoderAdapter::new().map_err(|error| error.to_string())?;
    let progressive_video = frame_sink.is_some();
    let video_send_started = Instant::now();
    for phase in 0..8u16 {
        let mut rgb = Vec::with_capacity(32 * 32 * 3);
        for y in 0..32 {
            for x in 0..32 {
                rgb.extend_from_slice(&[
                    (x * 7 + phase as usize * 13) as u8,
                    (y * 9 + phase as usize * 17) as u8,
                    ((x + y) * 4 + phase as usize * 23) as u8,
                ]);
            }
        }
        let encoded = h264_encoder
            .encode_rgb(&rgb, (32, 32))
            .map_err(|error| error.to_string())?;
        if phase == 2 {
            continue;
        }
        let video_rtp = amazing_webrtc::RtpPacket {
            marker: true,
            payload_type: video_payload_type,
            sequence_number: 200 + phase,
            timestamp: 90_000 + phase as u32 * 45_000,
            ssrc: 0x5649_4445,
            payload: encoded,
        };
        if progressive_video
            && let Some(remaining) =
                Duration::from_millis(phase as u64 * 500).checked_sub(video_send_started.elapsed())
        {
            std::thread::sleep(remaining);
        }
        endpoint
            .send_rtp(&amazing_webrtc::encode_rtp(&video_rtp).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        if phase == 3 {
            let feedback = endpoint.receive_rtcp().map_err(|error| error.to_string())?;
            let feedback = amazing_webrtc::parse_rtcp_picture_loss_indication(&feedback)
                .map_err(|error| error.to_string())?;
            if feedback.sender_ssrc != 0x5256_4352 || feedback.media_ssrc != video_rtp.ssrc {
                return Err("child returned inconsistent RTCP PLI".into());
            }
            h264_encoder.force_keyframe();
        }
    }

    read_control_marker(&mut control_reader, "PEER_OK")?;
    endpoint
        .close()
        .map_err(|error| format!("close parent secure endpoint: {error}"))?;
    drop(control_reader);
    drop(control);
    let status = child.finish(Duration::from_secs(5))?;
    if !status.success() {
        return Err(format!("secure peer exited {status}"));
    }
    Ok(())
}

fn run_secure_media_peer(
    control_address: Option<&str>,
    frame_sink: Option<&str>,
) -> Result<(), String> {
    let control_address =
        control_address.ok_or_else(|| "secure peer requires a control address".to_string())?;
    let control_address = control_address
        .parse()
        .map_err(|error| format!("parse signaling control address: {error}"))?;
    let mut control = TcpStream::connect_timeout(&control_address, Duration::from_secs(5))
        .map_err(|error| format!("connect signaling control: {error}"))?;
    configure_control_stream(&control)?;
    let mut frame_sink = frame_sink
        .map(|address| {
            let address = address
                .parse()
                .map_err(|error| format!("parse secure frame sink address: {error}"))?;
            let stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))
                .map_err(|error| format!("connect secure frame sink: {error}"))?;
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .map_err(|error| format!("configure secure frame sink: {error}"))?;
            Ok::<_, String>(stream)
        })
        .transpose()?;
    let mut control_reader =
        BufReader::new(control.try_clone().map_err(|error| error.to_string())?);
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")
        .map_err(|error| format!("bind child UDP: {error}"))?;
    let child_address = std_socket_v4(&socket)?;
    let identity = amazing_webrtc_crypto::SecureIdentity::generate("ferric-child")
        .map_err(|error| error.to_string())?;
    let offer = amazing_webrtc_crypto::build_local_description(
        child_address,
        identity.fingerprint(),
        "actpass",
        "childhost",
        "child",
        "FerricChildPassword123456",
        300,
    )
    .map_err(|error| error.to_string())?;
    write_signaled_description(&mut control, "OFFER", &offer)?;
    let answer = read_signaled_description(&mut control_reader, "ANSWER")?;
    let negotiated_codecs = amazing_webrtc::negotiate_audio_codecs(&offer, &answer)
        .map_err(|error| error.to_string())?;
    let negotiated_video = negotiated_video_media(&offer, &answer, "H264")?;
    if negotiated_video.section.mid != "1" || negotiated_video.section.media != "video" {
        return Err("H.264 payload did not demultiplex to bundled video".into());
    }
    let video_payload_type = negotiated_video.payload_type;
    let video_clock = negotiated_video.clock_rate;
    socket
        .connect(description_candidate(&answer)?)
        .map_err(|error| format!("connect child UDP: {error}"))?;
    let mut endpoint = amazing_webrtc_crypto::SecureUdpEndpoint::establish(
        amazing_webrtc_crypto::SecureEndpointConfig {
            socket,
            identity,
            local_description: offer,
            remote_description: answer,
            local_is_offerer: true,
            ice_role: amazing_webrtc_crypto::IceRole::Controlled,
        },
    )
    .map_err(|error| error.to_string())?;
    let parent_rtp = endpoint.receive_rtp().map_err(|error| error.to_string())?;
    let parent_rtp = amazing_webrtc::parse_rtp(&parent_rtp).map_err(|error| error.to_string())?;
    if parent_rtp.payload_type != 111 || parent_rtp.sequence_number != 41 {
        return Err("parent process sent unexpected RTP".into());
    }
    let parent_config = negotiated_codecs
        .jitter_config(parent_rtp.payload_type, parent_rtp.ssrc, 60, 8)
        .map_err(|error| error.to_string())?;
    let mut parent_jitter = amazing_webrtc::RtpJitterBuffer::new_adaptive(
        parent_config,
        amazing_webrtc::AdaptivePlayoutConfig {
            max_delay_ms: 200,
            jitter_multiplier: 4,
        },
    )
    .map_err(|error| error.to_string())?;
    parent_jitter
        .push(parent_rtp.clone(), 0)
        .map_err(|error| error.to_string())?;
    let Some(amazing_webrtc::JitterOutput::Packet(played_parent_rtp)) = parent_jitter.pop_ready(60)
    else {
        return Err("parent RTP did not reach jitter playout".into());
    };
    let mut parent_decoder = negotiated_audio_decoder(&negotiated_codecs, parent_rtp.payload_type)?;
    let parent_audio = parent_decoder
        .decode(&played_parent_rtp)
        .map_err(|error| error.to_string())?;
    if parent_audio.samples.len() != 960 {
        return Err("parent Opus playout sample count was incorrect".into());
    }
    if let Some(frame_sink) = frame_sink.as_mut() {
        write_secure_pcm_track(frame_sink, &parent_audio)?;
    }
    endpoint
        .answer_consent()
        .map_err(|error| error.to_string())?;
    let response = amazing_webrtc::RtpPacket {
        marker: true,
        payload_type: 0,
        sequence_number: 84,
        timestamp: 1_920,
        ssrc: 0x4348_494c,
        payload: vec![0xff; 160],
    };
    let response_wire = amazing_webrtc::encode_rtp(&response).map_err(|error| error.to_string())?;
    endpoint
        .send_rtp(&response_wire)
        .map_err(|error| error.to_string())?;
    let response_later = amazing_webrtc::RtpPacket {
        sequence_number: 86,
        timestamp: 2_240,
        payload: vec![0x80; 160],
        ..response.clone()
    };
    let response_later_wire =
        amazing_webrtc::encode_rtp(&response_later).map_err(|error| error.to_string())?;
    endpoint
        .send_rtp(&response_later_wire)
        .map_err(|error| error.to_string())?;
    let report = amazing_webrtc::RtcpSenderReport {
        ssrc: response.ssrc,
        ntp_timestamp: 0xe123_4567_89ab_cdef,
        rtp_timestamp: response_later.timestamp,
        packet_count: 2,
        octet_count: (response.payload.len() + response_later.payload.len()) as u32,
    };
    let compound_sender = amazing_webrtc::RtcpCompoundSenderReport {
        sender: report,
        cname: "child@ferric".into(),
    };
    let compound_sender = amazing_webrtc::encode_rtcp_compound_sender_report(&compound_sender)
        .map_err(|error| error.to_string())?;
    endpoint
        .send_rtcp(&compound_sender)
        .map_err(|error| error.to_string())?;
    writeln!(control, "READY_RR").map_err(|error| error.to_string())?;
    control.flush().map_err(|error| error.to_string())?;
    let receiver_report = match endpoint.receive_rtcp() {
        Ok(report) => report,
        Err(error) => {
            let _ = writeln!(
                control,
                "PEER_ERROR receive compound receiver report: {error}"
            );
            let _ = control.flush();
            return Err(format!("receive compound receiver report: {error}"));
        }
    };
    let receiver_report =
        match amazing_webrtc::parse_rtcp_compound_receiver_report(&receiver_report) {
            Ok(report) => report,
            Err(error) => {
                let _ = writeln!(
                    control,
                    "PEER_ERROR parse compound receiver report: {error}"
                );
                let _ = control.flush();
                return Err(error.to_string());
            }
        };
    if receiver_report.reception.source_ssrc != response.ssrc
        || receiver_report.reception.extended_highest_sequence
            != response_later.sequence_number as u32
        || receiver_report.reception.cumulative_lost != 1
        || receiver_report.reception.last_sender_report != (report.ntp_timestamp >> 16) as u32
        || receiver_report.cname != "parent@ferric"
    {
        return Err("parent process returned inconsistent compound RTCP".into());
    }
    let mut parent_reports = Vec::new();
    for _ in 0..2 {
        let wire = endpoint.receive_rtcp().map_err(|error| error.to_string())?;
        parent_reports.push(
            amazing_webrtc::parse_rtcp_compound_sender_report(&wire)
                .map_err(|error| error.to_string())?,
        );
    }
    let audio_sender = parent_reports
        .iter()
        .find(|report| report.sender.ssrc == parent_rtp.ssrc)
        .ok_or_else(|| "parent audio Sender Report is missing".to_string())?;
    let video_sender = parent_reports
        .iter()
        .find(|report| report.sender.ssrc == 0x5649_4445)
        .ok_or_else(|| "parent video Sender Report is missing".to_string())?;
    if audio_sender.cname != "parent-audio@amazing"
        || video_sender.cname != "parent-video@amazing"
        || video_sender.sender.packet_count != 0
        || video_sender.sender.rtp_timestamp != 90_000
    {
        return Err("parent A/V Sender Reports are inconsistent".into());
    }
    let av_sync = amazing_webrtc::AvSyncTimeline::new(
        amazing_webrtc::MediaClockAnchor::from_sender_report(audio_sender.sender, 48_000)
            .map_err(|error| error.to_string())?,
        amazing_webrtc::MediaClockAnchor::from_sender_report(video_sender.sender, 90_000)
            .map_err(|error| error.to_string())?,
        20,
    )
    .map_err(|error| error.to_string())?;
    let progressive_video = frame_sink.is_some();
    let mut video_jitter =
        amazing_webrtc::RtpJitterBuffer::new(amazing_webrtc::JitterBufferConfig {
            ssrc: 0x5649_4445,
            payload_type: video_payload_type,
            clock_rate: video_clock,
            playout_delay_ms: 30,
            capacity: 16,
        })
        .map_err(|error| error.to_string())?;
    let mut assembler =
        amazing_webrtc::VideoFrameAssembler::new(amazing_webrtc::VideoFrameConfig {
            ssrc: 0x5649_4445,
            payload_type: video_payload_type,
            max_packets: 8,
            max_bytes: 1_000_000,
        })
        .map_err(|error| error.to_string())?;
    let mut decoder =
        amazing_webrtc_codecs::H264DecoderAdapter::new().map_err(|error| error.to_string())?;
    let mut playout = amazing_webrtc::VideoPlayoutQueue::new(amazing_webrtc::VideoPlayoutConfig {
        clock_rate: video_clock,
        delay_ms: 30,
        capacity: 8,
    })
    .map_err(|error| error.to_string())?;
    let mut video_output = SecureVideoOutput::new(&av_sync, parent_rtp.timestamp, &mut frame_sink);
    let mut pli_sent = false;
    let expected_sequences = [200, 201, 203, 204, 205, 206, 207];
    let expected_timestamps = [90_000, 135_000, 225_000, 270_000, 315_000, 360_000, 405_000];
    for received in 0..7usize {
        let video_wire = endpoint.receive_rtp().map_err(|error| error.to_string())?;
        let video_rtp =
            amazing_webrtc::parse_rtp(&video_wire).map_err(|error| error.to_string())?;
        if video_rtp.payload_type != video_payload_type
            || video_rtp.ssrc != 0x5649_4445
            || video_rtp.sequence_number != expected_sequences[received]
            || video_rtp.timestamp != expected_timestamps[received]
        {
            return Err("secure H.264 RTP sequence or timestamp was incorrect".into());
        }
        let arrival_ms =
            video_rtp.timestamp.wrapping_sub(90_000) as u64 * 1_000 / video_clock as u64;
        video_jitter
            .push(video_rtp, arrival_ms)
            .map_err(|error| error.to_string())?;
        if progressive_video {
            std::thread::sleep(Duration::from_millis(30));
        }
        while let Some(output) = video_jitter.pop_ready(arrival_ms.saturating_add(30)) {
            match assembler.push(output).map_err(|error| error.to_string())? {
                amazing_webrtc::VideoAssembly::Frame(frame) => {
                    let timestamp = frame.timestamp;
                    let decoded = decoder
                        .decode(&frame.payload)
                        .map_err(|error| error.to_string())?;
                    let playout_arrival_ms = if progressive_video { arrival_ms } else { 0 };
                    if playout.push(timestamp, decoded, playout_arrival_ms)
                        != amazing_webrtc::VideoQueuePush::Queued
                    {
                        return Err("secure decoded video frame was not queued".into());
                    }
                }
                amazing_webrtc::VideoAssembly::Dropped if !pli_sent => {
                    endpoint
                        .send_rtcp(&amazing_webrtc::encode_rtcp_picture_loss_indication(
                            amazing_webrtc::RtcpPictureLossIndication {
                                sender_ssrc: 0x5256_4352,
                                media_ssrc: 0x5649_4445,
                            },
                        ))
                        .map_err(|error| error.to_string())?;
                    pli_sent = true;
                }
                _ => {}
            }
        }
        if progressive_video {
            while let Some(frame) = playout.pop_due(arrival_ms.saturating_add(30)) {
                video_output.present(frame)?;
            }
        }
    }
    while let Some(output) = video_jitter.pop_ready(4_000) {
        if let amazing_webrtc::VideoAssembly::Frame(frame) =
            assembler.push(output).map_err(|error| error.to_string())?
        {
            let timestamp = frame.timestamp;
            let decoded = decoder
                .decode(&frame.payload)
                .map_err(|error| error.to_string())?;
            if playout.push(timestamp, decoded, 0) != amazing_webrtc::VideoQueuePush::Queued {
                return Err("secure decoded video frame was not queued".into());
            }
        }
    }
    while let Some(frame) = playout.pop_due(4_000) {
        video_output.present(frame)?;
    }
    if !pli_sent {
        return Err("secure H.264 recovery did not send RTCP PLI".into());
    }
    if video_jitter.stats().gaps != 1 {
        return Err("secure H.264 recovery did not observe exactly one RTP loss".into());
    }
    video_output.finish(playout.stats().emitted)?;
    endpoint
        .close()
        .map_err(|error| format!("close child secure endpoint: {error}"))?;
    writeln!(control, "PEER_OK").map_err(|error| error.to_string())?;
    control.flush().map_err(|error| error.to_string())?;
    Ok(())
}

struct SecureVideoOutput<'a> {
    av_sync: &'a amazing_webrtc::AvSyncTimeline,
    audio_base_timestamp: u32,
    av_origin_ns: i128,
    frame_sink: &'a mut Option<TcpStream>,
    frames: usize,
    first_checksum: Option<u64>,
    pixels_changed: bool,
    previous_timestamp: Option<u32>,
}

impl<'a> SecureVideoOutput<'a> {
    fn new(
        av_sync: &'a amazing_webrtc::AvSyncTimeline,
        audio_base_timestamp: u32,
        frame_sink: &'a mut Option<TcpStream>,
    ) -> Self {
        Self {
            av_sync,
            audio_base_timestamp,
            av_origin_ns: av_sync.audio.presentation_nanoseconds(audio_base_timestamp),
            frame_sink,
            frames: 0,
            first_checksum: None,
            pixels_changed: false,
            previous_timestamp: None,
        }
    }

    fn present(
        &mut self,
        frame: amazing_webrtc::TimedVideoFrame<amazing_webrtc_codecs::DecodedVideoFrame>,
    ) -> Result<(), String> {
        let presentation_ns = self
            .av_sync
            .video
            .presentation_nanoseconds(frame.timestamp)
            .saturating_sub(self.av_origin_ns);
        if presentation_ns < 0
            || presentation_ns / 1_000_000 > u32::MAX as i128
            || presentation_ns.saturating_mul(48_000) / 1_000_000_000 > u32::MAX as i128
        {
            return Err("secure frame presentation time is out of range".into());
        }
        let audio_timestamp = self
            .audio_base_timestamp
            .wrapping_add((presentation_ns.saturating_mul(48_000) / 1_000_000_000) as u32);
        if self
            .previous_timestamp
            .is_some_and(|previous| frame.timestamp <= previous)
            || (frame.frame.width, frame.frame.height) != (32, 32)
            || frame.frame.planes != 3
            || frame.frame.rgba.len() != 32 * 32 * 4
        {
            return Err("secure H.264 playout frame was invalid".into());
        }
        if self.av_sync.decide(audio_timestamp, frame.timestamp)
            != amazing_webrtc::AvSyncDecision::Present
        {
            return Err("secure H.264 frame was not aligned to audio master".into());
        }
        self.previous_timestamp = Some(frame.timestamp);
        match self.first_checksum {
            Some(first) if first != frame.frame.checksum => self.pixels_changed = true,
            None => self.first_checksum = Some(frame.frame.checksum),
            _ => {}
        }
        if let Some(frame_sink) = self.frame_sink.as_mut() {
            write_secure_rgba_frame(
                frame_sink,
                self.frames as u32,
                frame.timestamp,
                (presentation_ns / 1_000_000) as u32,
                &frame.frame,
            )?;
        }
        self.frames += 1;
        Ok(())
    }

    fn finish(&mut self, emitted: u64) -> Result<(), String> {
        if self.frames != 6 || !self.pixels_changed || emitted != 6 {
            return Err("secure H.264 clocked playout was incomplete".into());
        }
        if let Some(frame_sink) = self.frame_sink.as_mut() {
            frame_sink.flush().map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

fn write_secure_rgba_frame(
    stream: &mut TcpStream,
    index: u32,
    timestamp: u32,
    presentation_ms: u32,
    frame: &amazing_webrtc_codecs::DecodedVideoFrame,
) -> Result<(), String> {
    const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
    let expected = (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "secure RGBA frame dimensions overflow".to_string())?;
    if frame.rgba.len() != expected || expected > MAX_FRAME_BYTES {
        return Err("secure RGBA frame length is invalid".into());
    }
    stream
        .write_all(b"AMV3")
        .map_err(|error| error.to_string())?;
    for value in [
        index,
        frame.width,
        frame.height,
        timestamp,
        presentation_ms,
        expected as u32,
    ] {
        stream
            .write_all(&value.to_le_bytes())
            .map_err(|error| error.to_string())?;
    }
    stream
        .write_all(&frame.rgba)
        .map_err(|error| error.to_string())
}

fn write_secure_pcm_track(
    stream: &mut TcpStream,
    audio: &amazing_webrtc::DecodedAudioFrame,
) -> Result<(), String> {
    const TRACK_SECONDS: usize = 30;
    const MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;
    if audio.sample_rate != 48_000
        || !matches!(audio.channels, 1 | 2)
        || audio.samples.is_empty()
        || !audio.samples.len().is_multiple_of(audio.channels as usize)
    {
        return Err("secure PCM source is invalid".into());
    }
    let sample_frames = audio.sample_rate as usize * TRACK_SECONDS;
    let sample_count = sample_frames
        .checked_mul(audio.channels as usize)
        .ok_or_else(|| "secure PCM sample count overflow".to_string())?;
    let byte_length = sample_count
        .checked_mul(std::mem::size_of::<i16>())
        .ok_or_else(|| "secure PCM byte length overflow".to_string())?;
    if byte_length > MAX_AUDIO_BYTES || byte_length > u32::MAX as usize {
        return Err("secure PCM track exceeds the byte cap".into());
    }
    stream
        .write_all(b"AMA3")
        .map_err(|error| error.to_string())?;
    for value in [
        audio.sample_rate,
        audio.channels as u32,
        sample_frames as u32,
        byte_length as u32,
    ] {
        stream
            .write_all(&value.to_le_bytes())
            .map_err(|error| error.to_string())?;
    }
    let mut pcm = Vec::with_capacity(byte_length);
    for sample in audio.samples.iter().cycle().take(sample_count) {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    stream.write_all(&pcm).map_err(|error| error.to_string())
}

fn accept_control_connection(
    listener: &TcpListener,
    child: &mut ChildGuard,
    timeout: Duration,
) -> Result<TcpStream, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(format!("accept signaling control: {error}")),
        }
        if let Some(status) = child
            .child_mut()
            .try_wait()
            .map_err(|error| format!("poll secure peer before signaling: {error}"))?
        {
            return Err(format!(
                "secure peer exited before signaling connection: {status}"
            ));
        }
        if Instant::now() >= deadline {
            return Err("secure peer signaling connection timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn configure_control_stream(stream: &TcpStream) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("configure signaling blocking mode: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("configure signaling read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("configure signaling write timeout: {error}"))
}

fn read_control_marker(reader: &mut impl BufRead, expected: &str) -> Result<(), String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| format!("read control marker {expected}: {error}"))?;
    if line.trim() != expected {
        return Err(format!(
            "expected control marker {expected}, received {line:?}"
        ));
    }
    Ok(())
}

fn write_signaled_description(
    writer: &mut impl Write,
    label: &str,
    description: &amazing_webrtc::SdpDescription,
) -> Result<(), String> {
    let sdp = amazing_webrtc::encode_sdp(description).map_err(|error| error.to_string())?;
    writeln!(writer, "{label} {}", hex_encode(sdp.as_bytes()))
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn read_signaled_description(
    reader: &mut impl BufRead,
    label: &str,
) -> Result<amazing_webrtc::SdpDescription, String> {
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err(format!("missing {label} signaling line"));
    }
    let encoded = line
        .trim_end()
        .strip_prefix(&format!("{label} "))
        .ok_or_else(|| format!("invalid {label} signaling line"))?;
    let bytes = hex_decode(encoded)?;
    let sdp = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    amazing_webrtc::parse_sdp(&sdp).map_err(|error| error.to_string())
}

fn description_candidate(
    description: &amazing_webrtc::SdpDescription,
) -> Result<std::net::SocketAddrV4, String> {
    description
        .primary_media()
        .map_err(|error| error.to_string())?
        .candidates
        .first()
        .map(|candidate| candidate.address)
        .ok_or_else(|| "signaled SDP has no ICE candidate".to_string())
}

pub(crate) fn std_socket_v4(socket: &std::net::UdpSocket) -> Result<std::net::SocketAddrV4, String> {
    match socket.local_addr().map_err(|error| error.to_string())? {
        std::net::SocketAddr::V4(address) => Ok(address),
        std::net::SocketAddr::V6(_) => Err("secure process artifact requires IPv4".into()),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err("signaling hex has invalid length or encoding".into());
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII checked");
            u8::from_str_radix(pair, 16).map_err(|_| "signaling hex is invalid".to_string())
        })
        .collect()
}

/// Phase 9.3 artifact: STUN discovery followed by one RTP echo over UDP.
pub(crate) fn cmd_webrtc_loopback() -> ExitCode {
    fn run() -> Result<
        (
            std::net::SocketAddrV4,
            amazing_webrtc::RtpPacket,
            amazing_webrtc::ChecklistState,
            amazing_webrtc::RtcpSenderReport,
        ),
        String,
    > {
        use std::net::{SocketAddr, UdpSocket};
        use std::time::Duration;

        let server = UdpSocket::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let server_address = server.local_addr().map_err(|error| error.to_string())?;
        let SocketAddr::V4(server_v4) = server_address else {
            return Err("loopback artifact requires IPv4".into());
        };
        let transaction = amazing_webrtc::TransactionId(*b"ferric-stun!");
        let ice_key = b"ferric-loopback-password";
        let client = UdpSocket::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let SocketAddr::V4(client_address) =
            client.local_addr().map_err(|error| error.to_string())?
        else {
            return Err("loopback artifact requires IPv4".into());
        };
        let local_candidate = amazing_webrtc::IceCandidate {
            foundation: "local-host".into(),
            component: 1,
            priority: 100,
            address: client_address,
            kind: amazing_webrtc::IceCandidateKind::Host,
            related_address: None,
        };
        let remote_candidate = amazing_webrtc::IceCandidate {
            foundation: "remote-host".into(),
            component: 1,
            priority: 80,
            address: server_v4,
            kind: amazing_webrtc::IceCandidateKind::Host,
            related_address: None,
        };
        let mut checklist =
            amazing_webrtc::IceChecklist::new(&[local_candidate], &[remote_candidate], true);
        let check_index = checklist
            .next_check()
            .ok_or_else(|| "ICE checklist had no candidate pair".to_string())?;
        let request = amazing_webrtc::encode_ice_binding_request(
            transaction,
            "remote:local",
            100,
            0x1020_3040_5060_7080,
            true,
            ice_key,
        )
        .map_err(|error| error.to_string())?;
        client
            .send_to(&request, server_address)
            .map_err(|error| error.to_string())?;
        let mut server_buffer = [0u8; 1500];
        let (length, peer) = server
            .recv_from(&mut server_buffer)
            .map_err(|error| error.to_string())?;
        let request = amazing_webrtc::parse_stun(&server_buffer[..length])
            .map_err(|error| error.to_string())?;
        if !amazing_webrtc::verify_message_integrity(&server_buffer[..length], ice_key)
            .map_err(|error| error.to_string())?
            || !amazing_webrtc::verify_fingerprint(&server_buffer[..length])
                .map_err(|error| error.to_string())?
        {
            return Err("ICE request authentication failed".into());
        }
        if request.class != amazing_webrtc::StunClass::Request
            || request.transaction_id != transaction
        {
            return Err("unexpected STUN Binding request".into());
        }
        if request.username.as_deref() != Some("remote:local")
            || request.priority != Some(100)
            || request.ice_controlling != Some(0x1020_3040_5060_7080)
            || request.ice_controlled.is_some()
            || !request.use_candidate
        {
            return Err("ICE request attributes failed semantic validation".into());
        }
        let SocketAddr::V4(peer_v4) = peer else {
            return Err("loopback artifact requires IPv4".into());
        };
        let response = amazing_webrtc::encode_ice_binding_success(transaction, peer_v4, ice_key)
            .map_err(|error| error.to_string())?;
        server
            .send_to(&response, peer)
            .map_err(|error| error.to_string())?;

        let mut client_buffer = [0u8; 1500];
        let (length, source) = client
            .recv_from(&mut client_buffer)
            .map_err(|error| error.to_string())?;
        if source != server_address {
            return Err("STUN response came from the wrong endpoint".into());
        }
        let response = amazing_webrtc::parse_stun(&client_buffer[..length])
            .map_err(|error| error.to_string())?;
        if !amazing_webrtc::verify_message_integrity(&client_buffer[..length], ice_key)
            .map_err(|error| error.to_string())?
            || !amazing_webrtc::verify_fingerprint(&client_buffer[..length])
                .map_err(|error| error.to_string())?
        {
            return Err("ICE response authentication failed".into());
        }
        if response.transaction_id != transaction
            || response.xor_mapped_address != Some(client_address)
        {
            return Err("STUN mapped address or transaction mismatch".into());
        }
        checklist
            .mark_succeeded(check_index, request.use_candidate)
            .map_err(|error| error.to_string())?;
        if checklist.state() != amazing_webrtc::ChecklistState::Completed {
            return Err("ICE checklist did not complete after nomination".into());
        }

        let outbound = amazing_webrtc::RtpPacket {
            marker: true,
            payload_type: 111,
            sequence_number: 7,
            timestamp: 960,
            ssrc: 0xf3_22_10_01,
            payload: b"ferric-loopback".to_vec(),
        };
        let wire = amazing_webrtc::encode_rtp(&outbound).map_err(|error| error.to_string())?;
        client
            .send_to(&wire, server_address)
            .map_err(|error| error.to_string())?;
        let (length, rtp_peer) = server
            .recv_from(&mut server_buffer)
            .map_err(|error| error.to_string())?;
        if rtp_peer != peer {
            return Err("RTP arrived from a different endpoint".into());
        }
        let received = amazing_webrtc::parse_rtp(&server_buffer[..length])
            .map_err(|error| error.to_string())?;
        server
            .send_to(&server_buffer[..length], peer)
            .map_err(|error| error.to_string())?;

        let (length, source) = client
            .recv_from(&mut client_buffer)
            .map_err(|error| error.to_string())?;
        if source != server_address {
            return Err("RTP echo came from the wrong endpoint".into());
        }
        let echoed = amazing_webrtc::parse_rtp(&client_buffer[..length])
            .map_err(|error| error.to_string())?;
        if echoed != outbound {
            return Err("RTP echo changed packet contents".into());
        }
        if received != outbound {
            return Err("loopback server decoded different RTP contents".into());
        }

        let sender_report = amazing_webrtc::RtcpSenderReport {
            ssrc: outbound.ssrc,
            ntp_timestamp: 0xe123_4567_89ab_cdef,
            rtp_timestamp: outbound.timestamp,
            packet_count: 1,
            octet_count: outbound.payload.len() as u32,
        };
        let rtcp = amazing_webrtc::encode_rtcp_sender_report(sender_report);
        client
            .send_to(&rtcp, server_address)
            .map_err(|error| error.to_string())?;
        let (length, rtcp_peer) = server
            .recv_from(&mut server_buffer)
            .map_err(|error| error.to_string())?;
        if rtcp_peer != peer {
            return Err("RTCP arrived from a different endpoint".into());
        }
        let server_report = amazing_webrtc::parse_rtcp_sender_report(&server_buffer[..length])
            .map_err(|error| error.to_string())?;
        server
            .send_to(&server_buffer[..length], peer)
            .map_err(|error| error.to_string())?;
        let (length, source) = client
            .recv_from(&mut client_buffer)
            .map_err(|error| error.to_string())?;
        if source != server_address {
            return Err("RTCP echo came from the wrong endpoint".into());
        }
        let echoed_report = amazing_webrtc::parse_rtcp_sender_report(&client_buffer[..length])
            .map_err(|error| error.to_string())?;
        if server_report != sender_report || echoed_report != sender_report {
            return Err("RTCP Sender Report changed during loopback".into());
        }
        Ok((client_address, echoed, checklist.state(), echoed_report))
    }

    match run() {
        Ok((mapped, packet, ice_state, report)) => {
            println!(
                "STUN mapped {mapped}; ICE state={ice_state:?} integrity=ok; RTP seq={} timestamp={} payload={} bytes; RTCP packets={} octets={}; loopback ok",
                packet.sequence_number,
                packet.timestamp,
                packet.payload.len(),
                report.packet_count,
                report.octet_count
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing webrtc-loopback: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 9.8 artifact: parse, canonicalize, and reparse a bounded SDP offer.
pub(crate) fn cmd_sdp_browser_answer(args: &[String]) -> ExitCode {
    let path = match args.first() {
        Some(path) => path,
        None => {
            eprintln!("usage: amazing sdp-browser-answer <offer.sdp>");
            return ExitCode::FAILURE;
        }
    };
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("amazing sdp-browser-answer: cannot read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let offer = match amazing_webrtc::parse_sdp(&source) {
        Ok(offer) => offer,
        Err(error) => {
            eprintln!("amazing sdp-browser-answer: parse offer: {error}");
            return ExitCode::FAILURE;
        }
    };
    let socket = match std::net::UdpSocket::bind("127.0.0.1:0") {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("amazing sdp-browser-answer: bind UDP: {error}");
            return ExitCode::FAILURE;
        }
    };
    let address = match std_socket_v4(&socket) {
        Ok(address) => address,
        Err(error) => {
            eprintln!("amazing sdp-browser-answer: {error}");
            return ExitCode::FAILURE;
        }
    };
    let identity = match amazing_webrtc_crypto::SecureIdentity::generate("amazing-browser-answer") {
        Ok(identity) => identity,
        Err(error) => {
            eprintln!("amazing sdp-browser-answer: identity: {error}");
            return ExitCode::FAILURE;
        }
    };
    let answer = match amazing_webrtc_crypto::build_local_answer(
        &offer,
        address,
        identity.fingerprint(),
        "browseranswer",
        "amaz",
        "AmazingBrowserPassword1234",
        900,
    ) {
        Ok(answer) => answer,
        Err(error) => {
            eprintln!("amazing sdp-browser-answer: {error}");
            return ExitCode::FAILURE;
        }
    };
    match amazing_webrtc::encode_sdp(&answer) {
        Ok(answer) => {
            print!("{answer}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing sdp-browser-answer: encode answer: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_browser_peer(port: Option<&str>) -> ExitCode {
    let port = match port.unwrap_or("8787").parse::<u16>() {
        Ok(port) if port > 0 => port,
        _ => {
            eprintln!("usage: amazing browser-peer [http-port]");
            return ExitCode::FAILURE;
        }
    };
    match browser_peer::run_browser_peer(port) {
        Ok(report) if !report.browser_result.starts_with("ERROR") => {
            println!(
                "BROWSER_INTEROP http={} browser={} media={} PT={} frames={} RTP={} result={}; browser peer ok",
                report.http_address,
                report.browser_address,
                report.local_media_address,
                report.payload_type,
                report.frames_sent,
                report.rtp_packets_sent,
                report.browser_result
            );
            ExitCode::SUCCESS
        }
        Ok(report) => {
            eprintln!("amazing browser-peer: {}", report.browser_result);
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("amazing browser-peer: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn cmd_sdp_roundtrip(args: &[String]) -> ExitCode {
    let path = match args.first() {
        Some(path) => path,
        None => {
            eprintln!("usage: amazing sdp-roundtrip <offer.sdp>");
            return ExitCode::FAILURE;
        }
    };
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("amazing sdp-roundtrip: cannot read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let parsed = match amazing_webrtc::parse_sdp(&source) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("amazing sdp-roundtrip: {error}");
            return ExitCode::FAILURE;
        }
    };
    let canonical = match amazing_webrtc::encode_sdp(&parsed) {
        Ok(canonical) => canonical,
        Err(error) => {
            eprintln!("amazing sdp-roundtrip: {error}");
            return ExitCode::FAILURE;
        }
    };
    match amazing_webrtc::parse_sdp(&canonical) {
        Ok(reparsed) if reparsed == parsed => {
            let media = parsed
                .media_sections
                .iter()
                .map(|section| section.media.as_str())
                .collect::<Vec<_>>()
                .join("+");
            let payloads = parsed
                .media_sections
                .iter()
                .map(|section| section.payload_types.len())
                .sum::<usize>();
            let candidates = parsed
                .media_sections
                .iter()
                .map(|section| section.candidates.len())
                .sum::<usize>();
            let rtp_maps = parsed
                .media_sections
                .iter()
                .map(|section| section.rtp_maps.len())
                .sum::<usize>();
            println!(
                "SDP {} bundle={} payloads={} candidates={} rtpmaps={} ICE={} setup={}; roundtrip ok",
                media,
                parsed.bundle.len(),
                payloads,
                candidates,
                rtp_maps,
                parsed.ice_ufrag,
                parsed.setup
            );
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("amazing sdp-roundtrip: canonical SDP changed semantics");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("amazing sdp-roundtrip: canonical SDP failed: {error}");
            ExitCode::FAILURE
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chromium_offer_selects_dynamic_opus_and_h264_payloads() {
        let offer =
            amazing_webrtc::parse_sdp(include_str!("../../../../examples/phase9-chromium-offer.sdp"))
                .unwrap();
        let audio = amazing_webrtc::negotiate_audio_codecs(&offer, &offer).unwrap();
        assert_eq!(audio.codec_by_encoding("opus").unwrap().payload_type, 111);
        let video = negotiated_video_media(&offer, &offer, "H264").unwrap();
        assert_eq!((video.payload_type, video.clock_rate), (102, 90_000));
        assert_eq!(
            video.section.direction,
            amazing_webrtc::SdpDirection::RecvOnly
        );
    }

}
