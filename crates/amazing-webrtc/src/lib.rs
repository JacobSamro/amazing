//! Phase 9.3 WebRTC transport foundations.
//!
//! STUN, RTP, and RTCP wire details stay behind small parse/encode interfaces. The
//! crate is safe, synchronous, and `std`-only. Authenticated ICE-style checks
//! follow RFC 8489/8445 wire rules and RFC 5769 vectors; TURN, DTLS/SRTP,
//! congestion control, and compressed media codecs are later sub-phases. The
//! `jitter` module provides bounded RTP playout plus L16 and PCMU audio decoder
//! adapters.

#![forbid(unsafe_code)]

pub mod jitter;
pub mod video;

pub use jitter::{
    AdaptivePlayoutConfig, DecodedAudioFrame, JitterBufferConfig, JitterCodecReport, JitterOutput,
    JitterPush, JitterStats, L16Decoder, NegotiatedAudioCodec, NegotiatedAudioCodecs, PcmuDecoder,
    RtpAudioDecoder, RtpJitterBuffer, negotiate_audio_codecs, run_jitter_codec_probe,
};
pub use video::{
    EncodedVideoFrame, VideoAssembly, VideoFrameAssembler, VideoFrameConfig, VideoFrameProbeReport,
    run_video_frame_probe,
};

use std::collections::HashSet;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};

const STUN_MAGIC_COOKIE: u32 = 0x2112_a442;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebRtcError {
    InvalidPacket(&'static str),
    Unsupported(&'static str),
}

impl fmt::Display for WebRtcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WebRtcError::InvalidPacket(reason) => write!(f, "invalid packet: {reason}"),
            WebRtcError::Unsupported(reason) => write!(f, "unsupported packet: {reason}"),
        }
    }
}

impl std::error::Error for WebRtcError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransactionId(pub [u8; 12]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StunClass {
    Request,
    SuccessResponse,
    ErrorResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StunMessage {
    pub class: StunClass,
    pub transaction_id: TransactionId,
    pub xor_mapped_address: Option<SocketAddrV4>,
    pub username: Option<String>,
    pub priority: Option<u32>,
    pub ice_controlling: Option<u64>,
    pub ice_controlled: Option<u64>,
    pub use_candidate: bool,
    pub error_code: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpPacket {
    pub marker: bool,
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtcpSenderReport {
    pub ssrc: u32,
    pub ntp_timestamp: u64,
    pub rtp_timestamp: u32,
    pub packet_count: u32,
    pub octet_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceptionReport {
    pub source_ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: i32,
    pub extended_highest_sequence: u32,
    pub interarrival_jitter: u32,
    pub last_sender_report: u32,
    pub delay_since_last_sender_report: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpCompoundReceiverReport {
    pub receiver_ssrc: u32,
    pub reception: ReceptionReport,
    pub cname: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpCompoundSenderReport {
    pub sender: RtcpSenderReport,
    pub cname: String,
}

/// RFC 3550 reception state for one RTP source.
pub struct RtpReceptionStats {
    source_ssrc: u32,
    base_sequence: Option<u16>,
    max_sequence: u16,
    sequence_cycles: u32,
    received: u32,
    expected_prior: u32,
    received_prior: u32,
    previous_transit: Option<i32>,
    jitter_q4: u32,
}

impl RtpReceptionStats {
    pub fn new(source_ssrc: u32) -> Self {
        Self {
            source_ssrc,
            base_sequence: None,
            max_sequence: 0,
            sequence_cycles: 0,
            received: 0,
            expected_prior: 0,
            received_prior: 0,
            previous_transit: None,
            jitter_q4: 0,
        }
    }

    /// `arrival_timestamp_units` uses the RTP payload clock rate.
    pub fn observe(
        &mut self,
        packet: &RtpPacket,
        arrival_timestamp_units: u32,
    ) -> Result<(), WebRtcError> {
        if packet.ssrc != self.source_ssrc {
            return Err(WebRtcError::InvalidPacket(
                "RTP packet SSRC does not match reception state",
            ));
        }
        match self.base_sequence {
            None => {
                self.base_sequence = Some(packet.sequence_number);
                self.max_sequence = packet.sequence_number;
            }
            Some(_) => {
                if packet.sequence_number < self.max_sequence
                    && self.max_sequence - packet.sequence_number > 0x8000
                {
                    self.sequence_cycles = self.sequence_cycles.saturating_add(1 << 16);
                    self.max_sequence = packet.sequence_number;
                } else if packet.sequence_number > self.max_sequence
                    && packet.sequence_number - self.max_sequence < 0x8000
                {
                    self.max_sequence = packet.sequence_number;
                }
            }
        }
        self.received = self.received.saturating_add(1);
        let transit = arrival_timestamp_units.wrapping_sub(packet.timestamp) as i32;
        if let Some(previous) = self.previous_transit {
            let difference = transit.wrapping_sub(previous).unsigned_abs();
            self.jitter_q4 = self
                .jitter_q4
                .saturating_add(difference)
                .saturating_sub((self.jitter_q4.saturating_add(8)) >> 4);
        }
        self.previous_transit = Some(transit);
        Ok(())
    }

    pub fn report(
        &mut self,
        last_sender_report: u32,
        delay_since_last_sender_report: u32,
    ) -> Result<ReceptionReport, WebRtcError> {
        let base_sequence = self
            .base_sequence
            .ok_or(WebRtcError::InvalidPacket("no RTP packets observed"))?;
        let extended_highest_sequence = self
            .sequence_cycles
            .saturating_add(self.max_sequence as u32);
        let expected = extended_highest_sequence
            .saturating_sub(base_sequence as u32)
            .saturating_add(1);
        let cumulative_lost_i64 = expected as i64 - self.received as i64;
        let cumulative_lost = cumulative_lost_i64.clamp(-8_388_608, 8_388_607) as i32;
        let expected_interval = expected.saturating_sub(self.expected_prior);
        let received_interval = self.received.saturating_sub(self.received_prior);
        let lost_interval = expected_interval as i64 - received_interval as i64;
        let fraction_lost = if expected_interval == 0 || lost_interval <= 0 {
            0
        } else {
            ((lost_interval as u64 * 256) / expected_interval as u64).min(255) as u8
        };
        self.expected_prior = expected;
        self.received_prior = self.received;
        Ok(ReceptionReport {
            source_ssrc: self.source_ssrc,
            fraction_lost,
            cumulative_lost,
            extended_highest_sequence,
            interarrival_jitter: self.jitter_q4 >> 4,
            last_sender_report,
            delay_since_last_sender_report,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpMap {
    pub payload_type: u8,
    pub encoding: String,
    pub clock_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdpMediaSection {
    pub mid: String,
    pub rtcp_mux: bool,
    pub connection_address: Ipv4Addr,
    pub media: String,
    pub port: u16,
    pub protocol: String,
    pub payload_types: Vec<u8>,
    pub candidates: Vec<IceCandidate>,
    pub rtp_maps: Vec<RtpMap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdpDescription {
    pub origin: String,
    pub session_name: String,
    pub timing: String,
    pub bundle: Vec<String>,
    pub ice_ufrag: String,
    pub ice_pwd: String,
    pub fingerprint_algorithm: String,
    pub fingerprint: String,
    pub setup: String,
    pub media_sections: Vec<SdpMediaSection>,
}

impl SdpDescription {
    pub fn media(&self, kind: &str) -> Option<&SdpMediaSection> {
        self.media_sections
            .iter()
            .find(|section| section.media == kind)
    }

    pub fn primary_media(&self) -> Result<&SdpMediaSection, WebRtcError> {
        self.media_sections
            .first()
            .ok_or(WebRtcError::InvalidPacket("SDP has no media section"))
    }

    pub fn media_by_mid(&self, mid: &str) -> Option<&SdpMediaSection> {
        self.media_sections
            .iter()
            .find(|section| section.mid == mid)
    }

    pub fn media_for_payload_type(
        &self,
        payload_type: u8,
    ) -> Result<&SdpMediaSection, WebRtcError> {
        let mut matches = self
            .media_sections
            .iter()
            .filter(|section| section.payload_types.contains(&payload_type));
        let section = matches.next().ok_or(WebRtcError::Unsupported(
            "SDP payload type was not negotiated",
        ))?;
        if matches.next().is_some() {
            return Err(WebRtcError::InvalidPacket(
                "SDP payload type is ambiguous across media sections",
            ));
        }
        Ok(section)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCandidate {
    pub foundation: String,
    pub component: u16,
    pub priority: u32,
    pub address: SocketAddrV4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidatePairState {
    Frozen,
    Waiting,
    InProgress,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidatePair {
    pub local: IceCandidate,
    pub remote: IceCandidate,
    pub priority: u64,
    pub state: CandidatePairState,
    pub nominated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecklistState {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentState {
    Fresh,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentPoll {
    WaitUntil(u64),
    SendNow,
    Expired,
}

/// RFC 7675 consent scheduling over a monotonic millisecond clock.
pub struct ConsentFreshness {
    last_success_ms: u64,
    next_check_ms: u64,
    outstanding: Vec<TransactionId>,
    used_transactions: HashSet<TransactionId>,
    expired: bool,
}

impl ConsentFreshness {
    pub const EXPIRY_MS: u64 = 30_000;
    pub const MIN_INTERVAL_MS: u64 = 4_000;
    pub const MAX_INTERVAL_MS: u64 = 6_000;
    pub const MAX_TRANSACTION_HISTORY: usize = 65_536;

    pub fn new(now_ms: u64, first_interval_ms: u64) -> Result<Self, WebRtcError> {
        Self::validate_interval(first_interval_ms)?;
        Ok(Self {
            last_success_ms: now_ms,
            next_check_ms: now_ms.saturating_add(first_interval_ms),
            outstanding: Vec::new(),
            used_transactions: HashSet::new(),
            expired: false,
        })
    }

    pub fn state(&self, now_ms: u64) -> ConsentState {
        if self.expired || now_ms.saturating_sub(self.last_success_ms) >= Self::EXPIRY_MS {
            ConsentState::Expired
        } else {
            ConsentState::Fresh
        }
    }

    pub fn poll(&mut self, now_ms: u64) -> ConsentPoll {
        if self.state(now_ms) == ConsentState::Expired {
            self.expired = true;
            self.outstanding.clear();
            ConsentPoll::Expired
        } else if now_ms >= self.next_check_ms {
            ConsentPoll::SendNow
        } else {
            ConsentPoll::WaitUntil(self.next_check_ms)
        }
    }

    /// Records a new transaction exactly once and schedules the next check.
    pub fn record_sent(
        &mut self,
        now_ms: u64,
        transaction: TransactionId,
        next_interval_ms: u64,
    ) -> Result<(), WebRtcError> {
        Self::validate_interval(next_interval_ms)?;
        match self.poll(now_ms) {
            ConsentPoll::SendNow => {}
            ConsentPoll::Expired => {
                return Err(WebRtcError::InvalidPacket("consent has expired"));
            }
            ConsentPoll::WaitUntil(_) => {
                return Err(WebRtcError::InvalidPacket(
                    "consent request was recorded before its due time",
                ));
            }
        }
        if self.outstanding.len() >= 8 {
            return Err(WebRtcError::InvalidPacket(
                "too many outstanding consent transactions",
            ));
        }
        if self.used_transactions.len() >= Self::MAX_TRANSACTION_HISTORY {
            return Err(WebRtcError::InvalidPacket(
                "consent transaction history exhausted; ICE restart required",
            ));
        }
        if !self.used_transactions.insert(transaction) {
            return Err(WebRtcError::InvalidPacket(
                "consent transaction identifier was reused",
            ));
        }
        self.outstanding.push(transaction);
        self.next_check_ms = now_ms.saturating_add(next_interval_ms);
        Ok(())
    }

    /// Accepts a matching authenticated response, including an older request.
    pub fn record_authenticated_response(
        &mut self,
        now_ms: u64,
        transaction: TransactionId,
    ) -> Result<bool, WebRtcError> {
        if self.poll(now_ms) == ConsentPoll::Expired {
            return Ok(false);
        }
        let Some(index) = self
            .outstanding
            .iter()
            .position(|candidate| *candidate == transaction)
        else {
            return Ok(false);
        };
        self.outstanding.swap_remove(index);
        self.last_success_ms = now_ms;
        Ok(true)
    }

    fn validate_interval(interval_ms: u64) -> Result<(), WebRtcError> {
        if !(Self::MIN_INTERVAL_MS..=Self::MAX_INTERVAL_MS).contains(&interval_ms) {
            return Err(WebRtcError::InvalidPacket(
                "consent interval must be between four and six seconds",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StunRetransmitPoll {
    WaitUntil(u64),
    SendNow { attempt: u8 },
    Succeeded,
    Failed,
    Cancelled,
}

/// RFC 8489 UDP transaction retransmission state on a monotonic clock.
pub struct StunRetransmission {
    rto_ms: u64,
    sends: u8,
    next_send_ms: u64,
    final_deadline_ms: u64,
    terminal: Option<StunRetransmitPoll>,
}

impl StunRetransmission {
    pub const DEFAULT_REQUEST_COUNT: u8 = 7;
    pub const FINAL_WAIT_MULTIPLIER: u64 = 16;

    pub fn new(now_ms: u64, rto_ms: u64) -> Result<Self, WebRtcError> {
        if rto_ms < 500 {
            return Err(WebRtcError::InvalidPacket("STUN RTO is below 500 ms"));
        }
        Ok(Self {
            rto_ms,
            sends: 0,
            next_send_ms: now_ms,
            final_deadline_ms: u64::MAX,
            terminal: None,
        })
    }

    pub fn poll(&self, now_ms: u64) -> StunRetransmitPoll {
        if let Some(terminal) = self.terminal {
            return terminal;
        }
        if self.sends < Self::DEFAULT_REQUEST_COUNT {
            if now_ms >= self.next_send_ms {
                StunRetransmitPoll::SendNow {
                    attempt: self.sends + 1,
                }
            } else {
                StunRetransmitPoll::WaitUntil(self.next_send_ms)
            }
        } else if now_ms >= self.final_deadline_ms {
            StunRetransmitPoll::Failed
        } else {
            StunRetransmitPoll::WaitUntil(self.final_deadline_ms)
        }
    }

    pub fn record_sent(&mut self, now_ms: u64) -> Result<(), WebRtcError> {
        if self.poll(now_ms)
            != (StunRetransmitPoll::SendNow {
                attempt: self.sends + 1,
            })
        {
            return Err(WebRtcError::InvalidPacket(
                "STUN retransmission was recorded outside its deadline",
            ));
        }
        self.sends += 1;
        if self.sends < Self::DEFAULT_REQUEST_COUNT {
            let exponent = self.sends - 1;
            let delay = self.rto_ms.saturating_mul(1u64 << exponent);
            self.next_send_ms = now_ms.saturating_add(delay);
        } else {
            self.final_deadline_ms =
                now_ms.saturating_add(self.rto_ms.saturating_mul(Self::FINAL_WAIT_MULTIPLIER));
        }
        Ok(())
    }

    pub fn succeed(&mut self, now_ms: u64) -> bool {
        if self.sends == 0
            || matches!(
                self.poll(now_ms),
                StunRetransmitPoll::Failed | StunRetransmitPoll::Cancelled
            )
        {
            return false;
        }
        self.terminal = Some(StunRetransmitPoll::Succeeded);
        true
    }

    pub fn cancel(&mut self, now_ms: u64) -> bool {
        if matches!(
            self.poll(now_ms),
            StunRetransmitPoll::Succeeded
                | StunRetransmitPoll::Failed
                | StunRetransmitPoll::Cancelled
        ) {
            return false;
        }
        self.terminal = Some(StunRetransmitPoll::Cancelled);
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceAgentRole {
    Controlling,
    Controlled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRoleAttribute {
    Controlling(u64),
    Controlled(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IceRoleConflict {
    Accept,
    Reject487,
    SwitchTo(IceAgentRole),
}

pub fn resolve_ice_role_conflict(
    local_role: IceAgentRole,
    local_tiebreaker: u64,
    remote: IceRoleAttribute,
) -> IceRoleConflict {
    match (local_role, remote) {
        (IceAgentRole::Controlling, IceRoleAttribute::Controlling(remote_tiebreaker)) => {
            if local_tiebreaker >= remote_tiebreaker {
                IceRoleConflict::Reject487
            } else {
                IceRoleConflict::SwitchTo(IceAgentRole::Controlled)
            }
        }
        (IceAgentRole::Controlled, IceRoleAttribute::Controlled(remote_tiebreaker)) => {
            if local_tiebreaker >= remote_tiebreaker {
                IceRoleConflict::SwitchTo(IceAgentRole::Controlling)
            } else {
                IceRoleConflict::Reject487
            }
        }
        _ => IceRoleConflict::Accept,
    }
}

pub fn role_after_487(sent: IceRoleAttribute) -> IceAgentRole {
    match sent {
        IceRoleAttribute::Controlling(_) => IceAgentRole::Controlled,
        IceRoleAttribute::Controlled(_) => IceAgentRole::Controlling,
    }
}

pub struct IceChecklist {
    pub pairs: Vec<CandidatePair>,
    local_is_controlling: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveIceRobustnessReport {
    pub request_sends: u8,
    pub injected_drops: u8,
    pub role_switched: bool,
    pub peer_reflexive_inserted: bool,
}

impl IceChecklist {
    pub fn new(
        local: &[IceCandidate],
        remote: &[IceCandidate],
        local_is_controlling: bool,
    ) -> Self {
        let mut pairs = Vec::new();
        for local_candidate in local {
            for remote_candidate in remote {
                if local_candidate.component != remote_candidate.component {
                    continue;
                }
                let (controlling, controlled) = if local_is_controlling {
                    (local_candidate.priority, remote_candidate.priority)
                } else {
                    (remote_candidate.priority, local_candidate.priority)
                };
                pairs.push(CandidatePair {
                    local: local_candidate.clone(),
                    remote: remote_candidate.clone(),
                    priority: candidate_pair_priority(controlling, controlled),
                    state: CandidatePairState::Frozen,
                    nominated: false,
                });
            }
        }
        pairs.sort_by_key(|pair| std::cmp::Reverse(pair.priority));
        if let Some(first) = pairs.first_mut() {
            first.state = CandidatePairState::Waiting;
        }
        Self {
            pairs,
            local_is_controlling,
        }
    }

    pub fn next_check(&mut self) -> Option<usize> {
        if self.state() != ChecklistState::Running {
            return None;
        }
        let index = self
            .pairs
            .iter()
            .position(|pair| pair.state == CandidatePairState::Waiting)
            .or_else(|| {
                self.pairs
                    .iter()
                    .position(|pair| pair.state == CandidatePairState::Frozen)
            })?;
        self.pairs[index].state = CandidatePairState::InProgress;
        Some(index)
    }

    pub fn mark_succeeded(&mut self, index: usize, nominated: bool) -> Result<(), WebRtcError> {
        let pair = self
            .pairs
            .get_mut(index)
            .ok_or(WebRtcError::InvalidPacket("ICE pair index is out of range"))?;
        if pair.state != CandidatePairState::InProgress {
            return Err(WebRtcError::InvalidPacket(
                "ICE success requires an InProgress pair",
            ));
        }
        pair.state = CandidatePairState::Succeeded;
        pair.nominated |= nominated;
        Ok(())
    }

    pub fn mark_failed(&mut self, index: usize) -> Result<(), WebRtcError> {
        let pair = self
            .pairs
            .get_mut(index)
            .ok_or(WebRtcError::InvalidPacket("ICE pair index is out of range"))?;
        if pair.state != CandidatePairState::InProgress {
            return Err(WebRtcError::InvalidPacket(
                "ICE failure requires an InProgress pair",
            ));
        }
        pair.state = CandidatePairState::Failed;
        Ok(())
    }

    pub fn state(&self) -> ChecklistState {
        if self.pairs.is_empty() {
            return ChecklistState::Failed;
        }
        let mut components = Vec::new();
        for pair in &self.pairs {
            if !components.contains(&pair.local.component) {
                components.push(pair.local.component);
            }
        }
        let complete = components.iter().all(|component| {
            self.pairs.iter().any(|pair| {
                pair.local.component == *component
                    && pair.state == CandidatePairState::Succeeded
                    && pair.nominated
            })
        });
        if complete {
            return ChecklistState::Completed;
        }
        if self.pairs.iter().all(|pair| {
            matches!(
                pair.state,
                CandidatePairState::Succeeded | CandidatePairState::Failed
            )
        }) {
            ChecklistState::Failed
        } else {
            ChecklistState::Running
        }
    }

    /// Adds the remote candidate discovered from the source address of an
    /// authenticated check and schedules its pair as a triggered check.
    pub fn add_peer_reflexive_remote(
        &mut self,
        base_pair_index: usize,
        address: SocketAddrV4,
        priority: u32,
    ) -> Result<usize, WebRtcError> {
        let base = self
            .pairs
            .get(base_pair_index)
            .ok_or(WebRtcError::InvalidPacket(
                "ICE base pair index is out of range",
            ))?
            .clone();
        if let Some(index) = self.pairs.iter().position(|pair| {
            pair.local == base.local
                && pair.remote.component == base.remote.component
                && pair.remote.address == address
        }) {
            if self.pairs[index].state == CandidatePairState::Frozen {
                self.pairs[index].state = CandidatePairState::Waiting;
            }
            return Ok(index);
        }
        let remote = IceCandidate {
            foundation: format!(
                "prflx-{}-{}-{}",
                base.remote.component,
                address.ip(),
                address.port()
            ),
            component: base.remote.component,
            priority,
            address,
        };
        let (controlling, controlled) = if self.local_is_controlling {
            (base.local.priority, remote.priority)
        } else {
            (remote.priority, base.local.priority)
        };
        self.pairs.push(CandidatePair {
            local: base.local,
            remote,
            priority: candidate_pair_priority(controlling, controlled),
            state: CandidatePairState::Waiting,
            nominated: false,
        });
        Ok(self.pairs.len() - 1)
    }

    pub fn set_local_role(&mut self, controlling: bool) {
        self.local_is_controlling = controlling;
        for pair in &mut self.pairs {
            let (controlling_priority, controlled_priority) = if controlling {
                (pair.local.priority, pair.remote.priority)
            } else {
                (pair.remote.priority, pair.local.priority)
            };
            pair.priority = candidate_pair_priority(controlling_priority, controlled_priority);
        }
    }
}

pub fn candidate_pair_priority(controlling: u32, controlled: u32) -> u64 {
    (1u64 << 32)
        .saturating_mul(controlling.min(controlled) as u64)
        .saturating_add(2u64.saturating_mul(controlling.max(controlled) as u64))
        .saturating_add(u64::from(controlling > controlled))
}

pub fn encode_binding_request(transaction_id: TransactionId) -> Vec<u8> {
    let mut packet = Vec::with_capacity(20);
    packet.extend_from_slice(&0x0001u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    packet.extend_from_slice(&transaction_id.0);
    packet
}

pub fn encode_binding_success(
    transaction_id: TransactionId,
    mapped_address: SocketAddrV4,
) -> Vec<u8> {
    let mut packet = Vec::with_capacity(32);
    packet.extend_from_slice(&0x0101u16.to_be_bytes());
    packet.extend_from_slice(&12u16.to_be_bytes());
    packet.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    packet.extend_from_slice(&transaction_id.0);
    packet.extend_from_slice(&0x0020u16.to_be_bytes());
    packet.extend_from_slice(&8u16.to_be_bytes());
    packet.push(0);
    packet.push(1);
    packet.extend_from_slice(
        &(mapped_address.port() ^ (STUN_MAGIC_COOKIE >> 16) as u16).to_be_bytes(),
    );
    let address = u32::from_be_bytes(mapped_address.ip().octets()) ^ STUN_MAGIC_COOKIE;
    packet.extend_from_slice(&address.to_be_bytes());
    packet
}

pub fn encode_ice_binding_request(
    transaction_id: TransactionId,
    username: &str,
    priority: u32,
    controlling_tie_breaker: u64,
    use_candidate: bool,
    key: &[u8],
) -> Result<Vec<u8>, WebRtcError> {
    encode_ice_binding_request_with_role(
        transaction_id,
        username,
        priority,
        IceRoleAttribute::Controlling(controlling_tie_breaker),
        use_candidate,
        key,
    )
}

pub fn encode_ice_binding_request_with_role(
    transaction_id: TransactionId,
    username: &str,
    priority: u32,
    role: IceRoleAttribute,
    use_candidate: bool,
    key: &[u8],
) -> Result<Vec<u8>, WebRtcError> {
    let mut packet = stun_header(0x0001, transaction_id);
    append_stun_attribute(&mut packet, 0x0006, username.as_bytes())?;
    append_stun_attribute(&mut packet, 0x0024, &priority.to_be_bytes())?;
    let (attribute, tie_breaker) = match role {
        IceRoleAttribute::Controlling(value) => (0x802a, value),
        IceRoleAttribute::Controlled(value) => (0x8029, value),
    };
    append_stun_attribute(&mut packet, attribute, &tie_breaker.to_be_bytes())?;
    if use_candidate {
        append_stun_attribute(&mut packet, 0x0025, &[])?;
    }
    finish_authenticated_stun(packet, key)
}

pub fn encode_ice_role_conflict_error(
    transaction_id: TransactionId,
    key: &[u8],
) -> Result<Vec<u8>, WebRtcError> {
    let mut packet = stun_header(0x0111, transaction_id);
    let mut error = vec![0, 0, 4, 87];
    error.extend_from_slice(b"Role Conflict");
    append_stun_attribute(&mut packet, 0x0009, &error)?;
    finish_authenticated_stun(packet, key)
}

/// Runs a real loopback UDP ICE exchange: two requests are intentionally
/// dropped, the third receives an authenticated 487, and a new controlled
/// retry succeeds. The receiver learns the request's source as a peer-
/// reflexive remote candidate.
pub fn run_live_ice_robustness_probe(
    replacement_tiebreaker: u64,
) -> Result<LiveIceRobustnessReport, WebRtcError> {
    use std::net::{SocketAddr, UdpSocket};
    use std::time::Duration;

    let server = UdpSocket::bind("127.0.0.1:0")
        .map_err(|_| WebRtcError::InvalidPacket("cannot bind ICE probe server"))?;
    let client = UdpSocket::bind("127.0.0.1:0")
        .map_err(|_| WebRtcError::InvalidPacket("cannot bind ICE probe client"))?;
    server
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| WebRtcError::InvalidPacket("cannot configure ICE probe server"))?;
    client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| WebRtcError::InvalidPacket("cannot configure ICE probe client"))?;
    let SocketAddr::V4(server_address) = server
        .local_addr()
        .map_err(|_| WebRtcError::InvalidPacket("cannot read ICE probe server address"))?
    else {
        return Err(WebRtcError::Unsupported("ICE probe requires IPv4"));
    };
    let SocketAddr::V4(client_address) = client
        .local_addr()
        .map_err(|_| WebRtcError::InvalidPacket("cannot read ICE probe client address"))?
    else {
        return Err(WebRtcError::Unsupported("ICE probe requires IPv4"));
    };

    let local = IceCandidate {
        foundation: "probe-server".into(),
        component: 1,
        priority: 100,
        address: server_address,
    };
    let signaled_remote = IceCandidate {
        foundation: "probe-signaled".into(),
        component: 1,
        priority: 80,
        address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9),
    };
    let mut server_checklist = IceChecklist::new(&[local], &[signaled_remote], true);
    let key = b"live-ice-probe-password";
    if replacement_tiebreaker == 0 || replacement_tiebreaker == 10 {
        return Err(WebRtcError::InvalidPacket(
            "ICE replacement tie-breaker must be new and nonzero",
        ));
    }
    let transaction = TransactionId(*b"live-loss!!!");
    let send_times = [0, 500, 1_500];
    let mut retransmission = StunRetransmission::new(0, 500)?;
    let mut buffer = [0u8; 1024];
    let mut peer_reflexive_index = None;

    for (position, now_ms) in send_times.into_iter().enumerate() {
        retransmission.record_sent(now_ms)?;
        let request = encode_ice_binding_request_with_role(
            transaction,
            "server:client",
            90,
            IceRoleAttribute::Controlling(10),
            true,
            key,
        )?;
        client
            .send_to(&request, server_address)
            .map_err(|_| WebRtcError::InvalidPacket("cannot send ICE probe request"))?;
        let (length, source) = server
            .recv_from(&mut buffer)
            .map_err(|_| WebRtcError::InvalidPacket("cannot receive ICE probe request"))?;
        let SocketAddr::V4(source) = source else {
            return Err(WebRtcError::Unsupported("ICE probe requires IPv4"));
        };
        let parsed = parse_stun(&buffer[..length])?;
        if source != client_address
            || parsed.class != StunClass::Request
            || parsed.transaction_id != transaction
            || parsed.username.as_deref() != Some("server:client")
            || parsed.priority != Some(90)
            || parsed.ice_controlling != Some(10)
            || !parsed.use_candidate
            || !verify_message_integrity(&buffer[..length], key)?
            || !verify_fingerprint(&buffer[..length])?
        {
            return Err(WebRtcError::InvalidPacket(
                "ICE probe request authentication failed",
            ));
        }
        let index = server_checklist.add_peer_reflexive_remote(
            0,
            source,
            parsed.priority.ok_or(WebRtcError::InvalidPacket(
                "ICE probe request omitted PRIORITY",
            ))?,
        )?;
        peer_reflexive_index = Some(index);
        if position < 2 {
            continue;
        }
        let remote_role = parsed
            .ice_controlling
            .map(IceRoleAttribute::Controlling)
            .or_else(|| parsed.ice_controlled.map(IceRoleAttribute::Controlled))
            .ok_or(WebRtcError::InvalidPacket("ICE probe request omitted role"))?;
        if resolve_ice_role_conflict(IceAgentRole::Controlling, 20, remote_role)
            != IceRoleConflict::Reject487
        {
            return Err(WebRtcError::InvalidPacket("ICE probe did not require 487"));
        }
        let error = encode_ice_role_conflict_error(transaction, key)?;
        server
            .send_to(&error, source)
            .map_err(|_| WebRtcError::InvalidPacket("cannot send ICE probe 487"))?;
    }

    let (length, error_source) = client
        .recv_from(&mut buffer)
        .map_err(|_| WebRtcError::InvalidPacket("cannot receive ICE probe 487"))?;
    let error = parse_stun(&buffer[..length])?;
    if error_source != SocketAddr::V4(server_address)
        || error.class != StunClass::ErrorResponse
        || error.transaction_id != transaction
        || error.error_code != Some(487)
        || !verify_message_integrity(&buffer[..length], key)?
        || !verify_fingerprint(&buffer[..length])?
    {
        return Err(WebRtcError::InvalidPacket(
            "ICE probe 487 authentication failed",
        ));
    }
    retransmission.cancel(1_500);
    let client_role = role_after_487(IceRoleAttribute::Controlling(10));
    let retry_role = match client_role {
        IceAgentRole::Controlling => IceRoleAttribute::Controlling(replacement_tiebreaker),
        IceAgentRole::Controlled => IceRoleAttribute::Controlled(replacement_tiebreaker),
    };
    let retry_transaction = TransactionId(*b"live-retry!!");
    let retry = encode_ice_binding_request_with_role(
        retry_transaction,
        "server:client",
        90,
        retry_role,
        false,
        key,
    )?;
    client
        .send_to(&retry, server_address)
        .map_err(|_| WebRtcError::InvalidPacket("cannot send ICE probe retry"))?;
    let (length, source) = server
        .recv_from(&mut buffer)
        .map_err(|_| WebRtcError::InvalidPacket("cannot receive ICE probe retry"))?;
    let parsed_retry = parse_stun(&buffer[..length])?;
    if source != SocketAddr::V4(client_address)
        || parsed_retry.class != StunClass::Request
        || parsed_retry.transaction_id != retry_transaction
        || parsed_retry.username.as_deref() != Some("server:client")
        || parsed_retry.priority != Some(90)
        || parsed_retry.ice_controlled != Some(replacement_tiebreaker)
        || parsed_retry.use_candidate
        || !verify_message_integrity(&buffer[..length], key)?
        || !verify_fingerprint(&buffer[..length])?
    {
        return Err(WebRtcError::InvalidPacket(
            "ICE probe retry did not switch role",
        ));
    }
    let SocketAddr::V4(source_v4) = source else {
        return Err(WebRtcError::Unsupported("ICE probe requires IPv4"));
    };
    let success = encode_ice_binding_success(retry_transaction, source_v4, key)?;
    server
        .send_to(&success, source)
        .map_err(|_| WebRtcError::InvalidPacket("cannot send ICE probe success"))?;
    let (length, success_source) = client
        .recv_from(&mut buffer)
        .map_err(|_| WebRtcError::InvalidPacket("cannot receive ICE probe success"))?;
    let parsed_success = parse_stun(&buffer[..length])?;
    if success_source != SocketAddr::V4(server_address)
        || parsed_success.class != StunClass::SuccessResponse
        || parsed_success.transaction_id != retry_transaction
        || parsed_success.xor_mapped_address != Some(client_address)
        || !verify_message_integrity(&buffer[..length], key)?
        || !verify_fingerprint(&buffer[..length])?
    {
        return Err(WebRtcError::InvalidPacket(
            "ICE probe retry did not succeed",
        ));
    }
    Ok(LiveIceRobustnessReport {
        request_sends: 3,
        injected_drops: 2,
        role_switched: client_role == IceAgentRole::Controlled,
        peer_reflexive_inserted: peer_reflexive_index
            .is_some_and(|index| server_checklist.pairs[index].remote.address == client_address),
    })
}

pub fn encode_ice_binding_success(
    transaction_id: TransactionId,
    mapped_address: SocketAddrV4,
    key: &[u8],
) -> Result<Vec<u8>, WebRtcError> {
    let mut packet = stun_header(0x0101, transaction_id);
    let mut mapped = [0u8; 8];
    mapped[1] = 1;
    mapped[2..4]
        .copy_from_slice(&(mapped_address.port() ^ (STUN_MAGIC_COOKIE >> 16) as u16).to_be_bytes());
    let address = u32::from_be_bytes(mapped_address.ip().octets()) ^ STUN_MAGIC_COOKIE;
    mapped[4..8].copy_from_slice(&address.to_be_bytes());
    append_stun_attribute(&mut packet, 0x0020, &mapped)?;
    finish_authenticated_stun(packet, key)
}

fn stun_header(message_type: u16, transaction_id: TransactionId) -> Vec<u8> {
    let mut packet = Vec::with_capacity(20);
    packet.extend_from_slice(&message_type.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    packet.extend_from_slice(&transaction_id.0);
    packet
}

fn append_stun_attribute(
    packet: &mut Vec<u8>,
    attribute_type: u16,
    value: &[u8],
) -> Result<(), WebRtcError> {
    let length = u16::try_from(value.len())
        .map_err(|_| WebRtcError::InvalidPacket("STUN attribute is too large"))?;
    packet.extend_from_slice(&attribute_type.to_be_bytes());
    packet.extend_from_slice(&length.to_be_bytes());
    packet.extend_from_slice(value);
    while packet.len() % 4 != 0 {
        packet.push(0);
    }
    Ok(())
}

fn set_stun_body_length(packet: &mut [u8], total_len: usize) -> Result<(), WebRtcError> {
    let body_len = total_len
        .checked_sub(20)
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(WebRtcError::InvalidPacket("STUN message is too large"))?;
    packet[2..4].copy_from_slice(&body_len.to_be_bytes());
    Ok(())
}

fn finish_authenticated_stun(mut packet: Vec<u8>, key: &[u8]) -> Result<Vec<u8>, WebRtcError> {
    let integrity_end = packet
        .len()
        .checked_add(24)
        .ok_or(WebRtcError::InvalidPacket("STUN integrity length overflow"))?;
    set_stun_body_length(&mut packet, integrity_end)?;
    let integrity = hmac_sha1(key, &packet);
    append_stun_attribute(&mut packet, 0x0008, &integrity)?;

    let fingerprint_end = packet
        .len()
        .checked_add(8)
        .ok_or(WebRtcError::InvalidPacket(
            "STUN fingerprint length overflow",
        ))?;
    set_stun_body_length(&mut packet, fingerprint_end)?;
    let fingerprint = crc32(&packet) ^ 0x5354_554e;
    append_stun_attribute(&mut packet, 0x8028, &fingerprint.to_be_bytes())?;
    Ok(packet)
}

pub fn parse_stun(packet: &[u8]) -> Result<StunMessage, WebRtcError> {
    if packet.len() < 20 {
        return Err(WebRtcError::InvalidPacket("truncated STUN header"));
    }
    let message_type = u16::from_be_bytes([packet[0], packet[1]]);
    if message_type & 0xc000 != 0 {
        return Err(WebRtcError::InvalidPacket(
            "STUN type uses reserved high bits",
        ));
    }
    let body_len = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if body_len % 4 != 0 || packet.len() != 20 + body_len {
        return Err(WebRtcError::InvalidPacket("STUN body length mismatch"));
    }
    if u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]) != STUN_MAGIC_COOKIE {
        return Err(WebRtcError::InvalidPacket("wrong STUN magic cookie"));
    }
    let class = match message_type {
        0x0001 => StunClass::Request,
        0x0101 => StunClass::SuccessResponse,
        0x0111 => StunClass::ErrorResponse,
        _ => return Err(WebRtcError::Unsupported("STUN method or class")),
    };
    let mut transaction = [0; 12];
    transaction.copy_from_slice(&packet[8..20]);
    let mut xor_mapped_address = None;
    let mut username = None;
    let mut priority = None;
    let mut ice_controlling = None;
    let mut ice_controlled = None;
    let mut use_candidate = false;
    let mut error_code = None;
    let mut integrity_seen = false;
    let mut fingerprint_seen = false;
    let mut cursor = 20;
    while cursor < packet.len() {
        if cursor + 4 > packet.len() {
            return Err(WebRtcError::InvalidPacket(
                "truncated STUN attribute header",
            ));
        }
        let attribute_type = u16::from_be_bytes([packet[cursor], packet[cursor + 1]]);
        let attribute_len = u16::from_be_bytes([packet[cursor + 2], packet[cursor + 3]]) as usize;
        let value_start = cursor + 4;
        let value_end = value_start
            .checked_add(attribute_len)
            .ok_or(WebRtcError::InvalidPacket("STUN attribute length overflow"))?;
        if value_end > packet.len() {
            return Err(WebRtcError::InvalidPacket("truncated STUN attribute"));
        }
        if fingerprint_seen {
            return Err(WebRtcError::InvalidPacket("attribute follows FINGERPRINT"));
        }
        if integrity_seen && attribute_type != 0x8028 {
            return Err(WebRtcError::InvalidPacket(
                "unauthenticated attribute follows MESSAGE-INTEGRITY",
            ));
        }
        match attribute_type {
            0x0020 => {
                if xor_mapped_address.is_some() {
                    return Err(WebRtcError::InvalidPacket("duplicate XOR-MAPPED-ADDRESS"));
                }
                if attribute_len != 8 || packet[value_start + 1] != 1 {
                    return Err(WebRtcError::Unsupported("non-IPv4 XOR-MAPPED-ADDRESS"));
                }
                let port = u16::from_be_bytes([packet[value_start + 2], packet[value_start + 3]])
                    ^ (STUN_MAGIC_COOKIE >> 16) as u16;
                let encoded_address = u32::from_be_bytes([
                    packet[value_start + 4],
                    packet[value_start + 5],
                    packet[value_start + 6],
                    packet[value_start + 7],
                ]);
                xor_mapped_address = Some(SocketAddrV4::new(
                    Ipv4Addr::from(encoded_address ^ STUN_MAGIC_COOKIE),
                    port,
                ));
            }
            0x0006 => {
                if username.is_some() {
                    return Err(WebRtcError::InvalidPacket("duplicate USERNAME"));
                }
                username = Some(
                    std::str::from_utf8(&packet[value_start..value_end])
                        .map_err(|_| WebRtcError::InvalidPacket("USERNAME is not UTF-8"))?
                        .to_owned(),
                );
            }
            0x0024 => {
                if priority.is_some() || attribute_len != 4 {
                    return Err(WebRtcError::InvalidPacket("invalid or duplicate PRIORITY"));
                }
                priority = Some(u32::from_be_bytes(
                    packet[value_start..value_end]
                        .try_into()
                        .expect("priority length checked"),
                ));
            }
            0x802a => {
                if ice_controlling.is_some() || attribute_len != 8 {
                    return Err(WebRtcError::InvalidPacket(
                        "invalid or duplicate ICE-CONTROLLING",
                    ));
                }
                ice_controlling = Some(u64::from_be_bytes(
                    packet[value_start..value_end]
                        .try_into()
                        .expect("tie-breaker length checked"),
                ));
            }
            0x8029 => {
                if ice_controlled.is_some() || attribute_len != 8 {
                    return Err(WebRtcError::InvalidPacket(
                        "invalid or duplicate ICE-CONTROLLED",
                    ));
                }
                ice_controlled = Some(u64::from_be_bytes(
                    packet[value_start..value_end]
                        .try_into()
                        .expect("tie-breaker length checked"),
                ));
            }
            0x0025 => {
                if use_candidate || attribute_len != 0 {
                    return Err(WebRtcError::InvalidPacket(
                        "invalid or duplicate USE-CANDIDATE",
                    ));
                }
                use_candidate = true;
            }
            0x0009 => {
                if error_code.is_some() || attribute_len < 4 {
                    return Err(WebRtcError::InvalidPacket(
                        "invalid or duplicate ERROR-CODE",
                    ));
                }
                if packet[value_start] != 0
                    || packet[value_start + 1] != 0
                    || packet[value_start + 2] & 0xf8 != 0
                {
                    return Err(WebRtcError::InvalidPacket(
                        "ERROR-CODE reserved bits are nonzero",
                    ));
                }
                let class = packet[value_start + 2] & 0x07;
                let number = packet[value_start + 3];
                if !(3..=6).contains(&class) || number >= 100 {
                    return Err(WebRtcError::InvalidPacket("invalid ERROR-CODE value"));
                }
                error_code = Some(class as u16 * 100 + number as u16);
            }
            0x0008 => {
                if integrity_seen || attribute_len != 20 {
                    return Err(WebRtcError::InvalidPacket(
                        "invalid or duplicate MESSAGE-INTEGRITY",
                    ));
                }
                integrity_seen = true;
            }
            0x8028 => {
                if attribute_len != 4 || value_end != packet.len() {
                    return Err(WebRtcError::InvalidPacket("FINGERPRINT is not final"));
                }
                fingerprint_seen = true;
            }
            _ => {}
        }
        let padded_len = attribute_len
            .checked_add(3)
            .ok_or(WebRtcError::InvalidPacket(
                "STUN attribute padding overflow",
            ))?
            / 4
            * 4;
        cursor = value_start
            .checked_add(padded_len)
            .ok_or(WebRtcError::InvalidPacket("STUN attribute cursor overflow"))?;
    }
    if class == StunClass::ErrorResponse && error_code.is_none() {
        return Err(WebRtcError::InvalidPacket(
            "STUN error response has no ERROR-CODE",
        ));
    }
    if class != StunClass::ErrorResponse && error_code.is_some() {
        return Err(WebRtcError::InvalidPacket(
            "ERROR-CODE appears outside an error response",
        ));
    }
    if ice_controlling.is_some() && ice_controlled.is_some() {
        return Err(WebRtcError::InvalidPacket(
            "ICE-CONTROLLING and ICE-CONTROLLED appear together",
        ));
    }
    Ok(StunMessage {
        class,
        transaction_id: TransactionId(transaction),
        xor_mapped_address,
        username,
        priority,
        ice_controlling,
        ice_controlled,
        use_candidate,
        error_code,
    })
}

pub fn encode_rtp(packet: &RtpPacket) -> Result<Vec<u8>, WebRtcError> {
    if packet.payload_type > 127 {
        return Err(WebRtcError::InvalidPacket(
            "RTP payload type exceeds seven bits",
        ));
    }
    let mut wire = Vec::with_capacity(12 + packet.payload.len());
    wire.push(0x80);
    wire.push((if packet.marker { 0x80 } else { 0 }) | packet.payload_type);
    wire.extend_from_slice(&packet.sequence_number.to_be_bytes());
    wire.extend_from_slice(&packet.timestamp.to_be_bytes());
    wire.extend_from_slice(&packet.ssrc.to_be_bytes());
    wire.extend_from_slice(&packet.payload);
    Ok(wire)
}

pub fn parse_rtp(packet: &[u8]) -> Result<RtpPacket, WebRtcError> {
    if packet.len() < 12 {
        return Err(WebRtcError::InvalidPacket("truncated RTP header"));
    }
    if packet[0] >> 6 != 2 {
        return Err(WebRtcError::InvalidPacket("RTP version is not 2"));
    }
    if packet[0] & 0x3f != 0 {
        return Err(WebRtcError::Unsupported(
            "RTP padding, extension, or CSRC list",
        ));
    }
    Ok(RtpPacket {
        marker: packet[1] & 0x80 != 0,
        payload_type: packet[1] & 0x7f,
        sequence_number: u16::from_be_bytes([packet[2], packet[3]]),
        timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
        payload: packet[12..].to_vec(),
    })
}

pub fn encode_rtcp_sender_report(report: RtcpSenderReport) -> Vec<u8> {
    let mut packet = Vec::with_capacity(28);
    packet.push(0x80);
    packet.push(200);
    packet.extend_from_slice(&6u16.to_be_bytes());
    packet.extend_from_slice(&report.ssrc.to_be_bytes());
    packet.extend_from_slice(&((report.ntp_timestamp >> 32) as u32).to_be_bytes());
    packet.extend_from_slice(&(report.ntp_timestamp as u32).to_be_bytes());
    packet.extend_from_slice(&report.rtp_timestamp.to_be_bytes());
    packet.extend_from_slice(&report.packet_count.to_be_bytes());
    packet.extend_from_slice(&report.octet_count.to_be_bytes());
    packet
}

pub fn parse_rtcp_sender_report(packet: &[u8]) -> Result<RtcpSenderReport, WebRtcError> {
    if packet.len() < 4 {
        return Err(WebRtcError::InvalidPacket("truncated RTCP header"));
    }
    if packet[0] >> 6 != 2 || packet[0] & 0x20 != 0 {
        return Err(WebRtcError::InvalidPacket(
            "RTCP version or padding is invalid",
        ));
    }
    if packet[0] & 0x1f != 0 {
        return Err(WebRtcError::Unsupported(
            "RTCP Sender Report reception blocks",
        ));
    }
    if packet[1] != 200 {
        return Err(WebRtcError::Unsupported(
            "RTCP packet is not a Sender Report",
        ));
    }
    let words_minus_one = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    let declared_len = words_minus_one
        .checked_add(1)
        .and_then(|words| words.checked_mul(4))
        .ok_or(WebRtcError::InvalidPacket("RTCP length overflow"))?;
    if declared_len != packet.len() || packet.len() != 28 {
        return Err(WebRtcError::InvalidPacket(
            "RTCP Sender Report length mismatch",
        ));
    }
    let word = |offset| {
        u32::from_be_bytes(
            packet[offset..offset + 4]
                .try_into()
                .expect("RTCP length checked"),
        )
    };
    Ok(RtcpSenderReport {
        ssrc: word(4),
        ntp_timestamp: ((word(8) as u64) << 32) | word(12) as u64,
        rtp_timestamp: word(16),
        packet_count: word(20),
        octet_count: word(24),
    })
}

/// Encodes an RFC 3550 compound RTCP packet: RR(1 block) + SDES(CNAME).
pub fn encode_rtcp_compound_receiver_report(
    report: &RtcpCompoundReceiverReport,
) -> Result<Vec<u8>, WebRtcError> {
    if !(-8_388_608..=8_388_607).contains(&report.reception.cumulative_lost) {
        return Err(WebRtcError::InvalidPacket(
            "RTCP cumulative loss exceeds signed 24 bits",
        ));
    }
    let mut wire = Vec::with_capacity(48 + report.cname.len());
    wire.extend_from_slice(&[0x81, 201]);
    wire.extend_from_slice(&7u16.to_be_bytes());
    wire.extend_from_slice(&report.receiver_ssrc.to_be_bytes());
    wire.extend_from_slice(&report.reception.source_ssrc.to_be_bytes());
    wire.push(report.reception.fraction_lost);
    let cumulative = (report.reception.cumulative_lost as u32) & 0x00ff_ffff;
    wire.extend_from_slice(&cumulative.to_be_bytes()[1..]);
    wire.extend_from_slice(&report.reception.extended_highest_sequence.to_be_bytes());
    wire.extend_from_slice(&report.reception.interarrival_jitter.to_be_bytes());
    wire.extend_from_slice(&report.reception.last_sender_report.to_be_bytes());
    wire.extend_from_slice(
        &report
            .reception
            .delay_since_last_sender_report
            .to_be_bytes(),
    );

    append_sdes_cname(&mut wire, report.receiver_ssrc, &report.cname)?;
    Ok(wire)
}

/// Parses the bounded compound RR(1 block) + SDES(CNAME) shape above.
pub fn parse_rtcp_compound_receiver_report(
    packet: &[u8],
) -> Result<RtcpCompoundReceiverReport, WebRtcError> {
    if packet.len() < 44 || packet[0] != 0x81 || packet[1] != 201 {
        return Err(WebRtcError::InvalidPacket(
            "compound RTCP must start with an unpadded one-block RR",
        ));
    }
    let rr_len = (u16::from_be_bytes([packet[2], packet[3]]) as usize + 1)
        .checked_mul(4)
        .ok_or(WebRtcError::InvalidPacket("RTCP RR length overflow"))?;
    if rr_len != 32 || packet.len() < rr_len + 12 {
        return Err(WebRtcError::InvalidPacket("RTCP RR length mismatch"));
    }
    let word = |offset| {
        u32::from_be_bytes(
            packet[offset..offset + 4]
                .try_into()
                .expect("RTCP word bounds checked"),
        )
    };
    let receiver_ssrc = word(4);
    let cumulative_raw = u32::from_be_bytes([0, packet[13], packet[14], packet[15]]);
    let cumulative_lost = if cumulative_raw & 0x0080_0000 != 0 {
        (cumulative_raw | 0xff00_0000) as i32
    } else {
        cumulative_raw as i32
    };
    let reception = ReceptionReport {
        source_ssrc: word(8),
        fraction_lost: packet[12],
        cumulative_lost,
        extended_highest_sequence: word(16),
        interarrival_jitter: word(20),
        last_sender_report: word(24),
        delay_since_last_sender_report: word(28),
    };

    let cname = parse_sdes_cname(&packet[rr_len..], receiver_ssrc)?;
    Ok(RtcpCompoundReceiverReport {
        receiver_ssrc,
        reception,
        cname,
    })
}

pub fn encode_rtcp_compound_sender_report(
    report: &RtcpCompoundSenderReport,
) -> Result<Vec<u8>, WebRtcError> {
    let mut wire = encode_rtcp_sender_report(report.sender);
    append_sdes_cname(&mut wire, report.sender.ssrc, &report.cname)?;
    Ok(wire)
}

pub fn parse_rtcp_compound_sender_report(
    packet: &[u8],
) -> Result<RtcpCompoundSenderReport, WebRtcError> {
    if packet.len() < 40 || packet[1] != 200 {
        return Err(WebRtcError::InvalidPacket(
            "compound RTCP must start with a Sender Report",
        ));
    }
    let sender_len = (u16::from_be_bytes([packet[2], packet[3]]) as usize + 1)
        .checked_mul(4)
        .ok_or(WebRtcError::InvalidPacket("RTCP SR length overflow"))?;
    if sender_len != 28 || packet.len() < sender_len + 12 {
        return Err(WebRtcError::InvalidPacket(
            "compound RTCP SR length mismatch",
        ));
    }
    let sender = parse_rtcp_sender_report(&packet[..sender_len])?;
    let cname = parse_sdes_cname(&packet[sender_len..], sender.ssrc)?;
    Ok(RtcpCompoundSenderReport { sender, cname })
}

fn append_sdes_cname(wire: &mut Vec<u8>, ssrc: u32, cname: &str) -> Result<(), WebRtcError> {
    let cname = cname.as_bytes();
    if cname.is_empty() || cname.len() > u8::MAX as usize || cname.contains(&0) {
        return Err(WebRtcError::InvalidPacket("invalid RTCP SDES CNAME"));
    }
    let sdes_start = wire.len();
    wire.extend_from_slice(&[0x81, 202, 0, 0]);
    wire.extend_from_slice(&ssrc.to_be_bytes());
    wire.push(1);
    wire.push(cname.len() as u8);
    wire.extend_from_slice(cname);
    wire.push(0);
    while (wire.len() - sdes_start) % 4 != 0 {
        wire.push(0);
    }
    let words_minus_one = (wire.len() - sdes_start) / 4 - 1;
    let words_minus_one = u16::try_from(words_minus_one)
        .map_err(|_| WebRtcError::InvalidPacket("RTCP SDES packet is too long"))?;
    wire[sdes_start + 2..sdes_start + 4].copy_from_slice(&words_minus_one.to_be_bytes());
    Ok(())
}

fn parse_sdes_cname(sdes: &[u8], expected_ssrc: u32) -> Result<String, WebRtcError> {
    if sdes.len() < 12 || sdes[0] != 0x81 || sdes[1] != 202 {
        return Err(WebRtcError::InvalidPacket(
            "compound RTCP report must be followed by one-chunk SDES",
        ));
    }
    let sdes_len = (u16::from_be_bytes([sdes[2], sdes[3]]) as usize + 1)
        .checked_mul(4)
        .ok_or(WebRtcError::InvalidPacket("RTCP SDES length overflow"))?;
    if sdes_len != sdes.len() {
        return Err(WebRtcError::InvalidPacket("RTCP SDES length mismatch"));
    }
    let sdes_ssrc = u32::from_be_bytes(sdes[4..8].try_into().expect("SDES SSRC bounds checked"));
    if sdes_ssrc != expected_ssrc || sdes[8] != 1 {
        return Err(WebRtcError::InvalidPacket(
            "RTCP SDES CNAME chunk does not match report SSRC",
        ));
    }
    let cname_len = sdes[9] as usize;
    let cname_end = 10usize
        .checked_add(cname_len)
        .ok_or(WebRtcError::InvalidPacket("RTCP CNAME length overflow"))?;
    if cname_len == 0 || cname_end >= sdes.len() || sdes[cname_end] != 0 {
        return Err(WebRtcError::InvalidPacket("invalid RTCP SDES CNAME item"));
    }
    if sdes[cname_end + 1..].iter().any(|byte| *byte != 0) {
        return Err(WebRtcError::InvalidPacket("nonzero RTCP SDES padding"));
    }
    std::str::from_utf8(&sdes[10..cname_end])
        .map(str::to_owned)
        .map_err(|_| WebRtcError::InvalidPacket("RTCP CNAME is not UTF-8"))
}

pub fn parse_sdp(text: &str) -> Result<SdpDescription, WebRtcError> {
    fn ice_text(value: &str, minimum: usize, field: &'static str) -> Result<String, WebRtcError> {
        if !(minimum..=256).contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
        {
            return Err(WebRtcError::InvalidPacket(field));
        }
        Ok(value.to_owned())
    }

    let lines: Vec<&str> = text
        .split_terminator('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() < 5 || lines[0] != "v=0" {
        return Err(WebRtcError::InvalidPacket("SDP must start with v=0"));
    }
    let origin = lines[1]
        .strip_prefix("o=")
        .ok_or(WebRtcError::InvalidPacket("SDP origin is missing"))?;
    let origin_fields: Vec<&str> = origin.split_ascii_whitespace().collect();
    if origin_fields.len() != 6
        || origin_fields[1].parse::<u64>().is_err()
        || origin_fields[2].parse::<u64>().is_err()
        || origin_fields[3] != "IN"
        || origin_fields[4] != "IP4"
        || origin_fields[5].parse::<Ipv4Addr>().is_err()
    {
        return Err(WebRtcError::InvalidPacket("unsupported SDP origin"));
    }
    let session_name = lines[2]
        .strip_prefix("s=")
        .filter(|value| !value.is_empty())
        .ok_or(WebRtcError::InvalidPacket("SDP session name is missing"))?;
    let timing = lines[3]
        .strip_prefix("t=")
        .ok_or(WebRtcError::InvalidPacket("SDP timing is missing"))?;
    let timing_fields: Vec<&str> = timing.split_ascii_whitespace().collect();
    if timing_fields.len() != 2
        || timing_fields[0].parse::<u64>().is_err()
        || timing_fields[1].parse::<u64>().is_err()
    {
        return Err(WebRtcError::InvalidPacket("unsupported SDP timing"));
    }

    let mut media_sections = Vec::<SdpMediaSection>::new();
    let mut media_connections = Vec::<bool>::new();
    let mut bundle = Vec::<String>::new();
    let mut ice_ufrag = None;
    let mut ice_pwd = None;
    let mut fingerprint_algorithm = None;
    let mut fingerprint = None;
    let mut setup = None;

    for line in &lines[4..] {
        let line = *line;
        if line.starts_with("v=")
            || line.starts_with("o=")
            || line.starts_with("s=")
            || line.starts_with("t=")
        {
            return Err(WebRtcError::InvalidPacket(
                "duplicate or out-of-order SDP session field",
            ));
        }
        if let Some(value) = line.strip_prefix("a=group:BUNDLE ") {
            if !media_sections.is_empty() || !bundle.is_empty() {
                return Err(WebRtcError::InvalidPacket("invalid SDP BUNDLE group"));
            }
            bundle = value.split_ascii_whitespace().map(str::to_owned).collect();
            if bundle.is_empty() {
                return Err(WebRtcError::InvalidPacket("empty SDP BUNDLE group"));
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("m=") {
            let fields: Vec<&str> = value.split_ascii_whitespace().collect();
            if fields.len() < 4 {
                return Err(WebRtcError::InvalidPacket("malformed SDP media line"));
            }
            if fields[0] != "audio" && fields[0] != "video" {
                return Err(WebRtcError::Unsupported("SDP media kind"));
            }
            let port = fields[1]
                .parse()
                .ok()
                .filter(|value| *value > 0)
                .ok_or(WebRtcError::InvalidPacket("invalid SDP media port"))?;
            if fields[2] != "UDP/TLS/RTP/SAVPF" {
                return Err(WebRtcError::Unsupported("SDP media transport profile"));
            }
            let payload_types = fields[3..]
                .iter()
                .map(|field| {
                    field
                        .parse::<u8>()
                        .ok()
                        .filter(|value| *value <= 127)
                        .ok_or(WebRtcError::InvalidPacket("invalid SDP payload type"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            media_sections.push(SdpMediaSection {
                mid: String::new(),
                rtcp_mux: false,
                connection_address: Ipv4Addr::UNSPECIFIED,
                media: fields[0].to_owned(),
                port,
                protocol: fields[2].to_owned(),
                payload_types,
                candidates: Vec::new(),
                rtp_maps: Vec::new(),
            });
            media_connections.push(false);
            continue;
        }
        if let Some(value) = line.strip_prefix("c=") {
            let index = media_sections
                .len()
                .checked_sub(1)
                .ok_or(WebRtcError::InvalidPacket("session-level SDP connection"))?;
            let section = media_sections
                .last_mut()
                .ok_or(WebRtcError::InvalidPacket("session-level SDP connection"))?;
            if media_connections[index] {
                return Err(WebRtcError::InvalidPacket("invalid SDP connection line"));
            }
            let fields: Vec<&str> = value.split_ascii_whitespace().collect();
            if fields.len() != 3 || fields[0] != "IN" || fields[1] != "IP4" {
                return Err(WebRtcError::Unsupported("SDP connection form"));
            }
            section.connection_address = fields[2]
                .parse::<Ipv4Addr>()
                .map_err(|_| WebRtcError::Unsupported("non-IPv4 SDP connection"))?;
            media_connections[index] = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("a=ice-ufrag:") {
            ice_ufrag = Some(ice_text(value, 4, "invalid SDP ice-ufrag")?);
            continue;
        }
        if let Some(value) = line.strip_prefix("a=ice-pwd:") {
            ice_pwd = Some(ice_text(value, 22, "invalid SDP ice-pwd")?);
            continue;
        }
        if let Some(value) = line.strip_prefix("a=fingerprint:") {
            let (algorithm, digest) = value
                .split_once(' ')
                .ok_or(WebRtcError::InvalidPacket("malformed SDP fingerprint"))?;
            if algorithm != "sha-256" {
                return Err(WebRtcError::Unsupported("SDP fingerprint algorithm"));
            }
            let bytes: Vec<&str> = digest.split(':').collect();
            if bytes.len() != 32
                || bytes.iter().any(|byte| {
                    byte.len() != 2
                        || !byte
                            .bytes()
                            .all(|ch| ch.is_ascii_digit() || (b'A'..=b'F').contains(&ch))
                })
            {
                return Err(WebRtcError::InvalidPacket(
                    "invalid SHA-256 SDP fingerprint",
                ));
            }
            fingerprint_algorithm = Some(algorithm.to_owned());
            fingerprint = Some(digest.to_owned());
            continue;
        }
        if let Some(value) = line.strip_prefix("a=setup:") {
            if !matches!(value, "actpass" | "active" | "passive" | "holdconn") {
                return Err(WebRtcError::InvalidPacket("invalid SDP setup role"));
            }
            setup = Some(value.to_owned());
            continue;
        }
        if let Some(value) = line.strip_prefix("a=mid:") {
            let section = media_sections
                .last_mut()
                .ok_or(WebRtcError::InvalidPacket("session-level SDP mid"))?;
            if !section.mid.is_empty()
                || value.is_empty()
                || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return Err(WebRtcError::InvalidPacket("invalid SDP mid"));
            }
            section.mid = value.to_owned();
            continue;
        }
        if line == "a=rtcp-mux" {
            let section = media_sections
                .last_mut()
                .ok_or(WebRtcError::InvalidPacket("session-level rtcp-mux"))?;
            if section.rtcp_mux {
                return Err(WebRtcError::InvalidPacket("duplicate SDP rtcp-mux"));
            }
            section.rtcp_mux = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("a=candidate:") {
            let section = media_sections
                .last_mut()
                .filter(|_| media_connections.last() == Some(&true))
                .ok_or(WebRtcError::InvalidPacket("session-level ICE candidate"))?;
            let fields: Vec<&str> = value.split_ascii_whitespace().collect();
            if fields.len() != 8
                || !fields[2].eq_ignore_ascii_case("UDP")
                || fields[6] != "typ"
                || fields[7] != "host"
            {
                return Err(WebRtcError::Unsupported("SDP candidate form"));
            }
            if fields[0].len() > 32 {
                return Err(WebRtcError::InvalidPacket(
                    "candidate foundation is too long",
                ));
            }
            let component = fields[1]
                .parse()
                .ok()
                .filter(|value| (1..=256).contains(value))
                .ok_or(WebRtcError::InvalidPacket("invalid candidate component"))?;
            let priority = fields[3]
                .parse()
                .map_err(|_| WebRtcError::InvalidPacket("invalid candidate priority"))?;
            let address = fields[4]
                .parse::<Ipv4Addr>()
                .map_err(|_| WebRtcError::Unsupported("non-IPv4 SDP candidate"))?;
            let candidate_port = fields[5]
                .parse()
                .ok()
                .filter(|value| *value > 0)
                .ok_or(WebRtcError::InvalidPacket("invalid candidate port"))?;
            section.candidates.push(IceCandidate {
                foundation: ice_text(fields[0], 1, "invalid candidate foundation")?,
                component,
                priority,
                address: SocketAddrV4::new(address, candidate_port),
            });
            continue;
        }
        if let Some(value) = line.strip_prefix("a=rtpmap:") {
            let section = media_sections
                .last_mut()
                .filter(|_| media_connections.last() == Some(&true))
                .ok_or(WebRtcError::InvalidPacket("session-level rtpmap"))?;
            let (payload, encoding) = value
                .split_once(' ')
                .ok_or(WebRtcError::InvalidPacket("malformed SDP rtpmap"))?;
            let payload_type = payload
                .parse::<u8>()
                .map_err(|_| WebRtcError::InvalidPacket("invalid rtpmap payload type"))?;
            let parts: Vec<&str> = encoding.split('/').collect();
            if !(2..=3).contains(&parts.len()) || parts[0].is_empty() {
                return Err(WebRtcError::InvalidPacket("malformed rtpmap encoding"));
            }
            let clock_rate = parts[1]
                .parse()
                .ok()
                .filter(|value| *value > 0)
                .ok_or(WebRtcError::InvalidPacket("invalid rtpmap clock rate"))?;
            let channels = parts
                .get(2)
                .map(|value| value.parse::<u16>())
                .transpose()
                .map_err(|_| WebRtcError::InvalidPacket("invalid rtpmap channels"))?
                .unwrap_or(1);
            if channels == 0 || !section.payload_types.contains(&payload_type) {
                return Err(WebRtcError::InvalidPacket(
                    "rtpmap does not match media payloads",
                ));
            }
            section.rtp_maps.push(RtpMap {
                payload_type,
                encoding: parts[0].to_owned(),
                clock_rate,
                channels,
            });
        }
    }

    if media_sections.is_empty() {
        return Err(WebRtcError::InvalidPacket("SDP media section is missing"));
    }
    if media_sections
        .iter()
        .zip(&media_connections)
        .any(|(section, has_connection)| {
            !has_connection || section.payload_types.is_empty() || section.candidates.is_empty()
        })
    {
        return Err(WebRtcError::InvalidPacket(
            "SDP media section is incomplete",
        ));
    }
    if !bundle.is_empty() {
        let mids: HashSet<&str> = media_sections
            .iter()
            .map(|section| section.mid.as_str())
            .collect();
        if mids.len() != media_sections.len()
            || mids.contains("")
            || bundle.len() != media_sections.len()
            || bundle.iter().any(|mid| !mids.contains(mid.as_str()))
            || media_sections.iter().any(|section| !section.rtcp_mux)
        {
            return Err(WebRtcError::InvalidPacket(
                "SDP BUNDLE mids or rtcp-mux are inconsistent",
            ));
        }
    }

    Ok(SdpDescription {
        origin: origin.to_owned(),
        session_name: session_name.to_owned(),
        timing: timing.to_owned(),
        bundle,
        ice_ufrag: ice_ufrag.ok_or(WebRtcError::InvalidPacket("SDP ice-ufrag is missing"))?,
        ice_pwd: ice_pwd.ok_or(WebRtcError::InvalidPacket("SDP ice-pwd is missing"))?,
        fingerprint_algorithm: fingerprint_algorithm
            .ok_or(WebRtcError::InvalidPacket("SDP fingerprint is missing"))?,
        fingerprint: fingerprint.ok_or(WebRtcError::InvalidPacket("SDP fingerprint is missing"))?,
        setup: setup.ok_or(WebRtcError::InvalidPacket("SDP setup is missing"))?,
        media_sections,
    })
}

pub fn encode_sdp(description: &SdpDescription) -> Result<String, WebRtcError> {
    if description.ice_ufrag.len() > 32 {
        return Err(WebRtcError::InvalidPacket(
            "outbound SDP ice-ufrag exceeds 32 characters",
        ));
    }
    let mut output = String::new();
    output.push_str("v=0\r\n");
    output.push_str(&format!("o={}\r\n", description.origin));
    output.push_str(&format!("s={}\r\n", description.session_name));
    output.push_str(&format!("t={}\r\n", description.timing));
    output.push_str(&format!("a=ice-ufrag:{}\r\n", description.ice_ufrag));
    output.push_str(&format!("a=ice-pwd:{}\r\n", description.ice_pwd));
    output.push_str(&format!(
        "a=fingerprint:{} {}\r\n",
        description.fingerprint_algorithm, description.fingerprint
    ));
    output.push_str(&format!("a=setup:{}\r\n", description.setup));
    if !description.bundle.is_empty() {
        output.push_str(&format!(
            "a=group:BUNDLE {}\r\n",
            description.bundle.join(" ")
        ));
    }
    for section in &description.media_sections {
        output.push_str(&format!(
            "m={} {} {} {}\r\n",
            section.media,
            section.port,
            section.protocol,
            section
                .payload_types
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ));
        output.push_str(&format!("c=IN IP4 {}\r\n", section.connection_address));
        if !section.mid.is_empty() {
            output.push_str(&format!("a=mid:{}\r\n", section.mid));
        }
        if section.rtcp_mux {
            output.push_str("a=rtcp-mux\r\n");
        }
        for candidate in &section.candidates {
            output.push_str(&format!(
                "a=candidate:{} {} UDP {} {} {} typ host\r\n",
                candidate.foundation,
                candidate.component,
                candidate.priority,
                candidate.address.ip(),
                candidate.address.port()
            ));
        }
        for mapping in &section.rtp_maps {
            output.push_str(&format!(
                "a=rtpmap:{} {}/{}{}\r\n",
                mapping.payload_type,
                mapping.encoding,
                mapping.clock_rate,
                if mapping.channels == 1 {
                    String::new()
                } else {
                    format!("/{}", mapping.channels)
                }
            ));
        }
    }
    let parsed = parse_sdp(&output)?;
    if &parsed != description {
        return Err(WebRtcError::InvalidPacket(
            "SDP description cannot be represented by the supported subset",
        ));
    }
    Ok(output)
}

/// Verifies the STUN MESSAGE-INTEGRITY attribute with a short-term key.
pub fn verify_message_integrity(packet: &[u8], key: &[u8]) -> Result<bool, WebRtcError> {
    let attributes = stun_attribute_ranges(packet)?;
    let integrity_index = attributes
        .iter()
        .position(|(kind, _, _, _)| *kind == 0x0008)
        .ok_or(WebRtcError::InvalidPacket("missing MESSAGE-INTEGRITY"))?;
    let (_, offset, value_start, value_end) = attributes[integrity_index];
    if value_end - value_start != 20 {
        return Err(WebRtcError::InvalidPacket(
            "MESSAGE-INTEGRITY length is not 20",
        ));
    }
    for (position, (kind, _, _, end)) in attributes.iter().enumerate().skip(integrity_index + 1) {
        if *kind != 0x8028 || position + 1 != attributes.len() || *end != packet.len() {
            return Err(WebRtcError::InvalidPacket(
                "unauthenticated attribute follows MESSAGE-INTEGRITY",
            ));
        }
    }
    let adjusted_body_len = offset
        .checked_add(24)
        .and_then(|end| end.checked_sub(20))
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(WebRtcError::InvalidPacket(
            "MESSAGE-INTEGRITY offset overflow",
        ))?;
    let mut authenticated = packet[..offset].to_vec();
    authenticated[2..4].copy_from_slice(&adjusted_body_len.to_be_bytes());
    let computed = hmac_sha1(key, &authenticated);
    Ok(digest_eq_no_early_exit(
        &computed,
        &packet[value_start..value_end],
    ))
}

/// Verifies the STUN FINGERPRINT attribute.
pub fn verify_fingerprint(packet: &[u8]) -> Result<bool, WebRtcError> {
    let attributes = stun_attribute_ranges(packet)?;
    let (_, offset, value_start, value_end) = attributes
        .into_iter()
        .find(|(kind, _, _, _)| *kind == 0x8028)
        .ok_or(WebRtcError::InvalidPacket("missing FINGERPRINT"))?;
    if value_end - value_start != 4 || value_end != packet.len() {
        return Err(WebRtcError::InvalidPacket(
            "FINGERPRINT must be the final four-byte attribute",
        ));
    }
    let expected = u32::from_be_bytes(
        packet[value_start..value_end]
            .try_into()
            .expect("fingerprint length checked"),
    );
    Ok(crc32(&packet[..offset]) ^ 0x5354_554e == expected)
}

fn stun_attribute_ranges(packet: &[u8]) -> Result<Vec<(u16, usize, usize, usize)>, WebRtcError> {
    if packet.len() < 20 {
        return Err(WebRtcError::InvalidPacket("truncated STUN header"));
    }
    let body_len = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if body_len % 4 != 0 || packet.len() != 20 + body_len {
        return Err(WebRtcError::InvalidPacket("STUN body length mismatch"));
    }
    if u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]) != STUN_MAGIC_COOKIE {
        return Err(WebRtcError::InvalidPacket("wrong STUN magic cookie"));
    }
    let mut attributes = Vec::new();
    let mut offset = 20;
    while offset < packet.len() {
        if offset + 4 > packet.len() {
            return Err(WebRtcError::InvalidPacket(
                "truncated STUN attribute header",
            ));
        }
        let kind = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let length = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        let value_start = offset + 4;
        let value_end = value_start
            .checked_add(length)
            .ok_or(WebRtcError::InvalidPacket("STUN attribute length overflow"))?;
        if value_end > packet.len() {
            return Err(WebRtcError::InvalidPacket("truncated STUN attribute"));
        }
        attributes.push((kind, offset, value_start, value_end));
        let padded = length
            .checked_add(3)
            .ok_or(WebRtcError::InvalidPacket("STUN padding overflow"))?
            / 4
            * 4;
        offset = value_start
            .checked_add(padded)
            .ok_or(WebRtcError::InvalidPacket("STUN cursor overflow"))?;
        if offset > packet.len() {
            return Err(WebRtcError::InvalidPacket(
                "truncated STUN attribute padding",
            ));
        }
    }
    Ok(attributes)
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut key_block = [0u8; 64];
    if key.len() > key_block.len() {
        key_block[..20].copy_from_slice(&sha1(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend(key_block.iter().map(|byte| byte ^ 0x36));
    inner.extend_from_slice(message);
    let inner_hash = sha1(&inner);
    let mut outer = Vec::with_capacity(64 + inner_hash.len());
    outer.extend(key_block.iter().map(|byte| byte ^ 0x5c));
    outer.extend_from_slice(&inner_hash);
    sha1(&outer)
}

fn sha1(message: &[u8]) -> [u8; 20] {
    let bit_len = (message.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(message.len() + 72);
    padded.extend_from_slice(message);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut state = [
        0x6745_2301u32,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 80];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let start = index * 4;
            *word = u32::from_be_bytes(chunk[start..start + 4].try_into().expect("word size"));
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.iter().enumerate() {
            let (function, constant) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(function)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }
    let mut digest = [0u8; 20];
    for (chunk, value) in digest.chunks_exact_mut(4).zip(state) {
        chunk.copy_from_slice(&value.to_be_bytes());
    }
    digest
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

fn digest_eq_no_early_exit(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(input: &str) -> Vec<u8> {
        let compact: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
        compact
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                    .expect("valid hex")
            })
            .collect()
    }

    #[test]
    fn binding_request_round_trips_through_public_interface() {
        let transaction = TransactionId(*b"ferric-stun!");
        let packet = encode_binding_request(transaction);
        let parsed = parse_stun(&packet).expect("valid STUN Binding request");

        assert_eq!(packet.len(), 20);
        assert_eq!(parsed.class, StunClass::Request);
        assert_eq!(parsed.transaction_id, transaction);
        assert_eq!(parsed.xor_mapped_address, None);
    }

    #[test]
    fn binding_success_recovers_xor_mapped_ipv4_address() {
        let transaction = TransactionId(*b"ferric-stun!");
        let address: SocketAddrV4 = "192.0.2.45:3478".parse().expect("test address");
        let packet = encode_binding_success(transaction, address);
        let parsed = parse_stun(&packet).expect("valid STUN Binding success");

        assert_eq!(parsed.class, StunClass::SuccessResponse);
        assert_eq!(parsed.transaction_id, transaction);
        assert_eq!(parsed.xor_mapped_address, Some(address));
    }

    #[test]
    fn rtp_packet_round_trips_header_and_payload() {
        let packet = RtpPacket {
            marker: true,
            payload_type: 111,
            sequence_number: 65_530,
            timestamp: 0x1020_3040,
            ssrc: 0xaabb_ccdd,
            payload: b"ferric-audio".to_vec(),
        };
        let wire = encode_rtp(&packet).expect("encodable RTP packet");
        let parsed = parse_rtp(&wire).expect("valid RTP packet");

        assert_eq!(wire.len(), 12 + packet.payload.len());
        assert_eq!(parsed, packet);
    }

    #[test]
    fn rfc5769_request_integrity_and_fingerprint_verify() {
        let packet = hex("
            000100582112a442b7e7a701bc34d686fa87dfae
            802200105354554e207465737420636c69656e74
            002400046e0001ff
            80290008932ff9b151263b36
            000600096576746a3a68367659202020
            000800149aeaa70cbfd8cb56781ef2b5b2d3f249c1b571a2
            80280004e57a3bcf
            ");
        let key = b"VOkJxbRl1RmTxUk/WvJxBt";

        assert!(verify_message_integrity(&packet, key).expect("integrity parse"));
        assert!(verify_fingerprint(&packet).expect("fingerprint parse"));
    }

    #[test]
    fn authenticated_ice_request_verifies_and_detects_tampering() {
        let transaction = TransactionId(*b"ferric-ice!!");
        let key = b"local-ice-password";
        let packet = encode_ice_binding_request(
            transaction,
            "remote:local",
            1_845_504_767,
            0x1020_3040_5060_7080,
            true,
            key,
        )
        .expect("encodable ICE check");

        let parsed = parse_stun(&packet).expect("valid STUN");
        assert_eq!(parsed.transaction_id, transaction);
        assert_eq!(parsed.username.as_deref(), Some("remote:local"));
        assert_eq!(parsed.priority, Some(1_845_504_767));
        assert_eq!(parsed.ice_controlling, Some(0x1020_3040_5060_7080));
        assert!(parsed.use_candidate);
        assert!(verify_message_integrity(&packet, key).expect("integrity parse"));
        assert!(verify_fingerprint(&packet).expect("fingerprint parse"));

        let mut tampered = packet.clone();
        tampered[24] ^= 1;
        assert!(!verify_message_integrity(&tampered, key).expect("tampered integrity parse"));
        assert!(!verify_fingerprint(&tampered).expect("tampered fingerprint parse"));
    }

    #[test]
    fn authenticated_487_and_controlled_retry_round_trip() {
        let transaction = TransactionId(*b"ferric-487!!");
        let key = b"local-ice-password";
        let error =
            encode_ice_role_conflict_error(transaction, key).expect("encodable role conflict");
        let parsed = parse_stun(&error).expect("valid role conflict");
        assert_eq!(parsed.class, StunClass::ErrorResponse);
        assert_eq!(parsed.error_code, Some(487));
        assert!(verify_message_integrity(&error, key).unwrap());
        assert!(verify_fingerprint(&error).unwrap());
        let mut reserved = error.clone();
        reserved[26] |= 0x80;
        assert!(parse_stun(&reserved).is_err());

        let retry = encode_ice_binding_request_with_role(
            transaction,
            "remote:local",
            100,
            IceRoleAttribute::Controlled(9),
            false,
            key,
        )
        .expect("encodable controlled retry");
        let parsed_retry = parse_stun(&retry).expect("valid controlled retry");
        assert_eq!(parsed_retry.ice_controlling, None);
        assert_eq!(parsed_retry.ice_controlled, Some(9));
    }

    #[test]
    fn rejects_conflicting_ice_role_attributes() {
        let transaction = TransactionId(*b"hostile-role");
        let key = b"local-ice-password";
        let mut packet = stun_header(0x0001, transaction);
        append_stun_attribute(&mut packet, 0x0006, b"remote:local").unwrap();
        append_stun_attribute(&mut packet, 0x0024, &100u32.to_be_bytes()).unwrap();
        append_stun_attribute(&mut packet, 0x802a, &7u64.to_be_bytes()).unwrap();
        append_stun_attribute(&mut packet, 0x8029, &8u64.to_be_bytes()).unwrap();
        let packet = finish_authenticated_stun(packet, key).unwrap();
        assert!(matches!(
            parse_stun(&packet),
            Err(WebRtcError::InvalidPacket(
                "ICE-CONTROLLING and ICE-CONTROLLED appear together"
            ))
        ));
    }

    #[test]
    fn rejects_unauthenticated_attribute_after_message_integrity() {
        let key = b"local-ice-password";
        let packet = encode_ice_binding_request(
            TransactionId(*b"ferric-ice!!"),
            "remote:local",
            100,
            7,
            false,
            key,
        )
        .expect("encodable ICE check");
        let fingerprint_offset = packet.len() - 8;
        let mut forged = packet[..fingerprint_offset].to_vec();
        forged.extend_from_slice(&0x0024u16.to_be_bytes());
        forged.extend_from_slice(&4u16.to_be_bytes());
        forged.extend_from_slice(&999u32.to_be_bytes());
        forged.extend_from_slice(&packet[fingerprint_offset..]);
        let body_len = (forged.len() - 20) as u16;
        forged[2..4].copy_from_slice(&body_len.to_be_bytes());

        assert!(verify_message_integrity(&forged, key).is_err());
        assert!(parse_stun(&forged).is_err());
    }

    #[test]
    fn rtcp_sender_report_round_trips_rfc3550_sender_info() {
        let report = RtcpSenderReport {
            ssrc: 0xaabb_ccdd,
            ntp_timestamp: 0xe123_4567_89ab_cdef,
            rtp_timestamp: 96_000,
            packet_count: 321,
            octet_count: 45_678,
        };
        let wire = encode_rtcp_sender_report(report);
        let parsed = parse_rtcp_sender_report(&wire).expect("valid RTCP SR");

        assert_eq!(wire.len(), 28);
        assert_eq!(wire[0], 0x80);
        assert_eq!(wire[1], 200);
        assert_eq!(&wire[2..4], &6u16.to_be_bytes());
        assert_eq!(parsed, report);
    }

    #[test]
    fn minimal_ice_dtls_sdp_round_trips() {
        let offer = concat!(
            "v=0\r\n",
            "o=- 1 1 IN IP4 127.0.0.1\r\n",
            "s=Ferric\r\n",
            "t=0 0\r\n",
            "a=ice-ufrag:ferric\r\n",
            "a=ice-pwd:FerricPassword1234567890AB\r\n",
            "a=fingerprint:sha-256 AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA\r\n",
            "a=setup:actpass\r\n",
            "m=audio 5000 UDP/TLS/RTP/SAVPF 111\r\n",
            "c=IN IP4 0.0.0.0\r\n",
            "a=candidate:host1 1 UDP 2130706431 127.0.0.1 5000 typ host\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
        );
        let parsed = parse_sdp(offer).expect("supported SDP offer");

        let audio = parsed.media("audio").expect("audio section");
        assert_eq!(parsed.ice_ufrag, "ferric");
        assert_eq!(audio.candidates.len(), 1);
        assert_eq!(audio.rtp_maps[0].encoding, "opus");
        assert_eq!(audio.rtp_maps[0].clock_rate, 48_000);
        assert_eq!(
            parse_sdp(&encode_sdp(&parsed).expect("serializable SDP")).unwrap(),
            parsed
        );
    }

    #[test]
    fn audio_and_video_media_sections_round_trip_in_order() {
        let offer = concat!(
            "v=0\r\n",
            "o=- 1 1 IN IP4 127.0.0.1\r\n",
            "s=Amazing AV\r\n",
            "t=0 0\r\n",
            "a=ice-ufrag:ferric\r\n",
            "a=ice-pwd:FerricPassword1234567890AB\r\n",
            "a=fingerprint:sha-256 AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA\r\n",
            "a=setup:actpass\r\n",
            "a=group:BUNDLE 0 1\r\n",
            "m=audio 5000 UDP/TLS/RTP/SAVPF 111\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "a=mid:0\r\n",
            "a=rtcp-mux\r\n",
            "a=candidate:a 1 UDP 100 127.0.0.1 5000 typ host\r\n",
            "a=rtpmap:111 opus/48000/1\r\n",
            "m=video 5002 UDP/TLS/RTP/SAVPF 120 121\r\n",
            "c=IN IP4 127.0.0.1\r\n",
            "a=mid:1\r\n",
            "a=rtcp-mux\r\n",
            "a=candidate:v 1 UDP 90 127.0.0.1 5002 typ host\r\n",
            "a=rtpmap:120 VP8/90000\r\n",
            "a=rtpmap:121 H264/90000\r\n",
        );
        let parsed = parse_sdp(offer).expect("multi-media SDP");
        assert_eq!(parsed.media_sections.len(), 2);
        assert_eq!(parsed.media_sections[0].media, "audio");
        assert_eq!(parsed.media_sections[1].media, "video");
        assert_eq!(parsed.bundle, ["0", "1"]);
        assert_eq!(parsed.media_by_mid("1").unwrap().media, "video");
        assert_eq!(parsed.media_for_payload_type(120).unwrap().mid, "1");
        assert_eq!(parsed.media("video").unwrap().rtp_maps.len(), 2);
        assert_eq!(parse_sdp(&encode_sdp(&parsed).unwrap()).unwrap(), parsed);

        let mut missing_bundle_member = parsed.clone();
        missing_bundle_member.bundle.pop();
        assert!(encode_sdp(&missing_bundle_member).is_err());
        let mut duplicate_mid = parsed.clone();
        duplicate_mid.media_sections[1].mid = "0".into();
        assert!(encode_sdp(&duplicate_mid).is_err());
        let mut ambiguous_payload = parsed;
        ambiguous_payload.media_sections[0].payload_types.push(120);
        assert!(ambiguous_payload.media_for_payload_type(120).is_err());
    }

    #[test]
    fn rejects_sdp_outside_the_declared_dtls_rtp_subset() {
        let unsupported = concat!(
            "v=0\n",
            "a=ice-ufrag:ferric\n",
            "a=ice-pwd:FerricPassword1234567890AB\n",
            "a=fingerprint:sha-256 AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA:AA\n",
            "a=setup:actpass\n",
            "m=audio 5000 RTP/AVP 200\n",
        );

        assert!(parse_sdp(unsupported).is_err());
    }

    #[test]
    fn authenticated_ice_success_preserves_mapped_address() {
        let transaction = TransactionId(*b"ferric-ice!!");
        let address: SocketAddrV4 = "198.51.100.7:50000".parse().expect("test address");
        let key = b"remote-ice-password";
        let packet =
            encode_ice_binding_success(transaction, address, key).expect("encodable ICE success");
        let parsed = parse_stun(&packet).expect("valid ICE success");

        assert_eq!(parsed.xor_mapped_address, Some(address));
        assert!(verify_message_integrity(&packet, key).expect("integrity parse"));
        assert!(verify_fingerprint(&packet).expect("fingerprint parse"));
    }

    #[test]
    fn ice_checklist_orders_checks_and_completes_on_nomination() {
        let candidate = |foundation: &str, priority, port| IceCandidate {
            foundation: foundation.into(),
            component: 1,
            priority,
            address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
        };
        let local = [
            candidate("local-high", 100, 5000),
            candidate("local-low", 50, 5001),
        ];
        let remote = [candidate("remote", 80, 6000)];
        let mut checklist = IceChecklist::new(&local, &remote, true);

        assert_eq!(checklist.pairs.len(), 2);
        assert_eq!(
            checklist.pairs[0].priority,
            candidate_pair_priority(100, 80)
        );
        assert_eq!(checklist.pairs[0].state, CandidatePairState::Waiting);
        assert_eq!(checklist.pairs[1].state, CandidatePairState::Frozen);
        assert!(checklist.mark_succeeded(1, true).is_err());
        let selected = checklist.next_check().expect("highest-priority check");
        assert_eq!(selected, 0);
        assert_eq!(checklist.pairs[0].state, CandidatePairState::InProgress);
        checklist
            .mark_succeeded(selected, true)
            .expect("valid pair index");
        assert_eq!(checklist.state(), ChecklistState::Completed);
    }

    #[test]
    fn checklist_adds_and_deduplicates_peer_reflexive_pair() {
        let candidate = |foundation: &str, priority, port| IceCandidate {
            foundation: foundation.into(),
            component: 1,
            priority,
            address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
        };
        let local = [candidate("local", 100, 5000)];
        let remote = [candidate("signaled", 80, 6000)];
        let mut checklist = IceChecklist::new(&local, &remote, true);
        let address: SocketAddrV4 = "127.0.0.1:7000".parse().unwrap();
        let index = checklist
            .add_peer_reflexive_remote(0, address, 90)
            .expect("peer-reflexive pair");
        assert_eq!(checklist.pairs[index].remote.address, address);
        assert!(
            checklist.pairs[index]
                .remote
                .foundation
                .starts_with("prflx-")
        );
        assert_eq!(checklist.pairs[index].state, CandidatePairState::Waiting);
        assert_eq!(
            checklist.add_peer_reflexive_remote(0, address, 90).unwrap(),
            index
        );
        assert_eq!(checklist.pairs.len(), 2);
        let controlling_priority = checklist.pairs[index].priority;
        checklist.set_local_role(false);
        assert_ne!(checklist.pairs[index].priority, controlling_priority);
    }

    #[test]
    fn live_ice_probe_drops_packets_switches_role_and_learns_remote() {
        let report =
            run_live_ice_robustness_probe(0xfeed_face_cafe_beef).expect("live ICE robustness");
        assert_eq!(report.request_sends, 3);
        assert_eq!(report.injected_drops, 2);
        assert!(report.role_switched);
        assert!(report.peer_reflexive_inserted);
    }

    #[test]
    fn compound_receiver_report_round_trips_with_sdes_cname() {
        let report = RtcpCompoundReceiverReport {
            receiver_ssrc: 0x4348_494c,
            reception: ReceptionReport {
                source_ssrc: 0x5041_5245,
                fraction_lost: 3,
                cumulative_lost: -2,
                extended_highest_sequence: 65_577,
                interarrival_jitter: 17,
                last_sender_report: 0x1234_5678,
                delay_since_last_sender_report: 0x0001_8000,
            },
            cname: "child@ferric".into(),
        };
        let wire = encode_rtcp_compound_receiver_report(&report)
            .expect("encodable compound receiver report");
        let parsed =
            parse_rtcp_compound_receiver_report(&wire).expect("valid compound receiver report");

        assert_eq!(parsed, report);
        assert_eq!(wire[1], 201);
        let rr_length = (u16::from_be_bytes([wire[2], wire[3]]) as usize + 1) * 4;
        assert_eq!(wire[rr_length + 1], 202);
    }

    #[test]
    fn compound_sender_report_round_trips_with_sdes_cname() {
        let report = RtcpCompoundSenderReport {
            sender: RtcpSenderReport {
                ssrc: 0x4348_494c,
                ntp_timestamp: 0xe123_4567_89ab_cdef,
                rtp_timestamp: 1_920,
                packet_count: 1,
                octet_count: 15,
            },
            cname: "child@ferric".into(),
        };
        let wire =
            encode_rtcp_compound_sender_report(&report).expect("encodable compound sender report");
        assert_eq!(parse_rtcp_compound_sender_report(&wire).unwrap(), report);
        assert_eq!(wire[1], 200);
        assert_eq!(wire[29], 202);
    }

    #[test]
    fn consent_freshness_schedules_refresh_and_expires_after_thirty_seconds() {
        let mut consent = ConsentFreshness::new(0, 5_000).expect("valid consent timer");
        let transaction = TransactionId(*b"consent-one!");

        assert_eq!(consent.state(0), ConsentState::Fresh);
        assert_eq!(consent.poll(4_999), ConsentPoll::WaitUntil(5_000));
        assert!(consent.record_sent(4_999, transaction, 5_000).is_err());
        assert_eq!(consent.poll(5_000), ConsentPoll::SendNow);
        consent
            .record_sent(5_000, transaction, 5_500)
            .expect("new one-shot consent request");
        assert!(
            consent
                .record_authenticated_response(7_000, transaction)
                .expect("matching authenticated response")
        );
        assert_eq!(consent.poll(10_499), ConsentPoll::WaitUntil(10_500));
        assert_eq!(consent.poll(10_500), ConsentPoll::SendNow);
        assert!(consent.record_sent(10_500, transaction, 5_000).is_err());
        assert_eq!(consent.poll(36_999), ConsentPoll::SendNow);
        assert_eq!(consent.poll(37_000), ConsentPoll::Expired);
        assert_eq!(consent.state(37_000), ConsentState::Expired);
        assert!(
            !consent
                .record_authenticated_response(37_001, transaction)
                .expect("late response cannot restore expired consent")
        );
    }

    #[test]
    fn stun_retransmission_matches_default_rfc8489_schedule() {
        let mut transaction = StunRetransmission::new(0, 500).expect("valid RTO");
        assert!(!transaction.succeed(0));
        let send_times = [0, 500, 1_500, 3_500, 7_500, 15_500, 31_500];
        for (index, send_time) in send_times.into_iter().enumerate() {
            assert_eq!(
                transaction.poll(send_time),
                StunRetransmitPoll::SendNow {
                    attempt: index as u8 + 1
                }
            );
            transaction.record_sent(send_time).expect("scheduled send");
        }
        assert_eq!(
            transaction.poll(39_499),
            StunRetransmitPoll::WaitUntil(39_500)
        );
        assert_eq!(transaction.poll(39_500), StunRetransmitPoll::Failed);
        assert!(!transaction.succeed(39_500));
        assert!(!transaction.cancel(39_500));

        let mut succeeded = StunRetransmission::new(0, 500).unwrap();
        succeeded.record_sent(0).unwrap();
        assert!(succeeded.succeed(100));
        assert!(!succeeded.cancel(100));
        assert_eq!(succeeded.poll(100), StunRetransmitPoll::Succeeded);

        let mut cancelled = StunRetransmission::new(0, 500).unwrap();
        cancelled.record_sent(0).unwrap();
        assert!(cancelled.cancel(100));
        assert!(!cancelled.succeed(100));
        assert_eq!(cancelled.poll(100), StunRetransmitPoll::Cancelled);
    }

    #[test]
    fn ice_role_conflict_follows_tiebreaker_rules() {
        assert_eq!(
            resolve_ice_role_conflict(
                IceAgentRole::Controlling,
                10,
                IceRoleAttribute::Controlling(9)
            ),
            IceRoleConflict::Reject487
        );
        assert_eq!(
            resolve_ice_role_conflict(
                IceAgentRole::Controlling,
                10,
                IceRoleAttribute::Controlling(11)
            ),
            IceRoleConflict::SwitchTo(IceAgentRole::Controlled)
        );
        assert_eq!(
            resolve_ice_role_conflict(
                IceAgentRole::Controlled,
                10,
                IceRoleAttribute::Controlled(9)
            ),
            IceRoleConflict::SwitchTo(IceAgentRole::Controlling)
        );
        assert_eq!(
            resolve_ice_role_conflict(
                IceAgentRole::Controlled,
                10,
                IceRoleAttribute::Controlled(11)
            ),
            IceRoleConflict::Reject487
        );
        assert_eq!(
            resolve_ice_role_conflict(
                IceAgentRole::Controlled,
                10,
                IceRoleAttribute::Controlling(11)
            ),
            IceRoleConflict::Accept
        );
        assert_eq!(
            role_after_487(IceRoleAttribute::Controlling(10)),
            IceAgentRole::Controlled
        );
    }

    #[test]
    fn reception_statistics_compute_loss_sequence_and_jitter() {
        let packet = |sequence_number, timestamp| RtpPacket {
            marker: false,
            payload_type: 111,
            sequence_number,
            timestamp,
            ssrc: 0x4348_494c,
            payload: vec![0],
        };
        let mut stats = RtpReceptionStats::new(0x4348_494c);
        stats.observe(&packet(100, 0), 0).unwrap();
        stats.observe(&packet(101, 960), 1_000).unwrap();
        stats.observe(&packet(103, 2_880), 3_000).unwrap();

        let first = stats.report(0x1234_5678, 0x0001_8000).unwrap();
        assert_eq!(first.fraction_lost, 64);
        assert_eq!(first.cumulative_lost, 1);
        assert_eq!(first.extended_highest_sequence, 103);
        assert_eq!(first.interarrival_jitter, 7);
        assert_eq!(first.last_sender_report, 0x1234_5678);

        stats.observe(&packet(104, 3_840), 4_000).unwrap();
        let second = stats.report(0, 0).unwrap();
        assert_eq!(second.fraction_lost, 0);
        assert_eq!(second.cumulative_lost, 1);
        assert_eq!(second.extended_highest_sequence, 104);
        assert_eq!(second.interarrival_jitter, 9);
    }

    #[test]
    fn reception_statistics_handle_wrap_late_duplicates_and_dlsr() {
        let packet = |sequence_number| RtpPacket {
            marker: false,
            payload_type: 111,
            sequence_number,
            timestamp: sequence_number as u32 * 960,
            ssrc: 0x1122_3344,
            payload: vec![0],
        };
        let mut stats = RtpReceptionStats::new(0x1122_3344);
        for sequence in [65_534, 65_535, 0, 1] {
            let packet = packet(sequence);
            stats.observe(&packet, packet.timestamp).unwrap();
        }
        let first = stats.report(0x0102_0304, 0x0002_8000).unwrap();
        assert_eq!(first.extended_highest_sequence, 65_537);
        assert_eq!(first.cumulative_lost, 0);
        assert_eq!(first.delay_since_last_sender_report, 0x0002_8000);

        for sequence in [65_535, 1] {
            let packet = packet(sequence);
            stats.observe(&packet, packet.timestamp).unwrap();
        }
        let second = stats.report(0, 9).unwrap();
        assert_eq!(second.extended_highest_sequence, 65_537);
        assert_eq!(second.cumulative_lost, -2);
        assert_eq!(second.fraction_lost, 0);
        assert_eq!(second.delay_since_last_sender_report, 9);
    }
}
