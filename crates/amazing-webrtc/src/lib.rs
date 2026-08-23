//! Phase 9.3 WebRTC transport foundations.
//!
//! STUN and RTP wire details stay behind small parse/encode interfaces. The
//! crate is safe, synchronous, and `std`-only. Authenticated ICE-style checks
//! follow RFC 8489/8445 wire rules and RFC 5769 vectors; TURN, DTLS/SRTP,
//! congestion control, and media codecs are later sub-phases.

#![forbid(unsafe_code)]

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionId(pub [u8; 12]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StunClass {
    Request,
    SuccessResponse,
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

pub struct IceChecklist {
    pub pairs: Vec<CandidatePair>,
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
        pairs.sort_by(|left, right| right.priority.cmp(&left.priority));
        if let Some(first) = pairs.first_mut() {
            first.state = CandidatePairState::Waiting;
        }
        Self { pairs }
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
    packet.extend_from_slice(&(mapped_address.port() ^ (STUN_MAGIC_COOKIE >> 16) as u16).to_be_bytes());
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
    let mut packet = stun_header(0x0001, transaction_id);
    append_stun_attribute(&mut packet, 0x0006, username.as_bytes())?;
    append_stun_attribute(&mut packet, 0x0024, &priority.to_be_bytes())?;
    append_stun_attribute(
        &mut packet,
        0x802a,
        &controlling_tie_breaker.to_be_bytes(),
    )?;
    if use_candidate {
        append_stun_attribute(&mut packet, 0x0025, &[])?;
    }
    finish_authenticated_stun(packet, key)
}

pub fn encode_ice_binding_success(
    transaction_id: TransactionId,
    mapped_address: SocketAddrV4,
    key: &[u8],
) -> Result<Vec<u8>, WebRtcError> {
    let mut packet = stun_header(0x0101, transaction_id);
    let mut mapped = [0u8; 8];
    mapped[1] = 1;
    mapped[2..4].copy_from_slice(
        &(mapped_address.port() ^ (STUN_MAGIC_COOKIE >> 16) as u16).to_be_bytes(),
    );
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
        .ok_or(WebRtcError::InvalidPacket("STUN fingerprint length overflow"))?;
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
        return Err(WebRtcError::InvalidPacket("STUN type uses reserved high bits"));
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
    let mut integrity_seen = false;
    let mut fingerprint_seen = false;
    let mut cursor = 20;
    while cursor < packet.len() {
        if cursor + 4 > packet.len() {
            return Err(WebRtcError::InvalidPacket("truncated STUN attribute header"));
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
            },
            0x0006 => {
                if username.is_some() {
                    return Err(WebRtcError::InvalidPacket("duplicate USERNAME"));
                }
                username = Some(
                    std::str::from_utf8(&packet[value_start..value_end])
                        .map_err(|_| WebRtcError::InvalidPacket("USERNAME is not UTF-8"))?
                        .to_owned(),
                );
            },
            0x0024 => {
                if priority.is_some() || attribute_len != 4 {
                    return Err(WebRtcError::InvalidPacket("invalid or duplicate PRIORITY"));
                }
                priority = Some(u32::from_be_bytes(
                    packet[value_start..value_end]
                        .try_into()
                        .expect("priority length checked"),
                ));
            },
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
            },
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
            },
            0x0025 => {
                if use_candidate || attribute_len != 0 {
                    return Err(WebRtcError::InvalidPacket(
                        "invalid or duplicate USE-CANDIDATE",
                    ));
                }
                use_candidate = true;
            },
            0x0008 => {
                if integrity_seen || attribute_len != 20 {
                    return Err(WebRtcError::InvalidPacket(
                        "invalid or duplicate MESSAGE-INTEGRITY",
                    ));
                }
                integrity_seen = true;
            },
            0x8028 => {
                if attribute_len != 4 || value_end != packet.len() {
                    return Err(WebRtcError::InvalidPacket("FINGERPRINT is not final"));
                }
                fingerprint_seen = true;
            },
            _ => {},
        }
        let padded_len = attribute_len
            .checked_add(3)
            .ok_or(WebRtcError::InvalidPacket("STUN attribute padding overflow"))?
            / 4
            * 4;
        cursor = value_start
            .checked_add(padded_len)
            .ok_or(WebRtcError::InvalidPacket("STUN attribute cursor overflow"))?;
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
    })
}

pub fn encode_rtp(packet: &RtpPacket) -> Result<Vec<u8>, WebRtcError> {
    if packet.payload_type > 127 {
        return Err(WebRtcError::InvalidPacket("RTP payload type exceeds seven bits"));
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
        return Err(WebRtcError::Unsupported("RTP padding, extension, or CSRC list"));
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

/// Verifies the STUN MESSAGE-INTEGRITY attribute with a short-term key.
pub fn verify_message_integrity(packet: &[u8], key: &[u8]) -> Result<bool, WebRtcError> {
    let attributes = stun_attribute_ranges(packet)?;
    let integrity_index = attributes
        .iter()
        .position(|(kind, _, _, _)| *kind == 0x0008)
        .ok_or(WebRtcError::InvalidPacket("missing MESSAGE-INTEGRITY"))?;
    let (_, offset, value_start, value_end) = attributes[integrity_index];
    if value_end - value_start != 20 {
        return Err(WebRtcError::InvalidPacket("MESSAGE-INTEGRITY length is not 20"));
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
        .ok_or(WebRtcError::InvalidPacket("MESSAGE-INTEGRITY offset overflow"))?;
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
        return Err(WebRtcError::InvalidPacket("FINGERPRINT must be the final four-byte attribute"));
    }
    let expected = u32::from_be_bytes(
        packet[value_start..value_end]
            .try_into()
            .expect("fingerprint length checked"),
    );
    Ok(crc32(&packet[..offset]) ^ 0x5354_554e == expected)
}

fn stun_attribute_ranges(
    packet: &[u8],
) -> Result<Vec<(u16, usize, usize, usize)>, WebRtcError> {
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
            return Err(WebRtcError::InvalidPacket("truncated STUN attribute header"));
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
            return Err(WebRtcError::InvalidPacket("truncated STUN attribute padding"));
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
            words[index] = (words[index - 3]
                ^ words[index - 8]
                ^ words[index - 14]
                ^ words[index - 16])
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
        let packet = hex(
            "
            000100582112a442b7e7a701bc34d686fa87dfae
            802200105354554e207465737420636c69656e74
            002400046e0001ff
            80290008932ff9b151263b36
            000600096576746a3a68367659202020
            000800149aeaa70cbfd8cb56781ef2b5b2d3f249c1b571a2
            80280004e57a3bcf
            ",
        );
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
    fn authenticated_ice_success_preserves_mapped_address() {
        let transaction = TransactionId(*b"ferric-ice!!");
        let address: SocketAddrV4 = "198.51.100.7:50000".parse().expect("test address");
        let key = b"remote-ice-password";
        let packet = encode_ice_binding_success(transaction, address, key)
            .expect("encodable ICE success");
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
        let local = [candidate("local-high", 100, 5000), candidate("local-low", 50, 5001)];
        let remote = [candidate("remote", 80, 6000)];
        let mut checklist = IceChecklist::new(&local, &remote, true);

        assert_eq!(checklist.pairs.len(), 2);
        assert_eq!(checklist.pairs[0].priority, candidate_pair_priority(100, 80));
        assert_eq!(checklist.pairs[0].state, CandidatePairState::Waiting);
        assert_eq!(checklist.pairs[1].state, CandidatePairState::Frozen);
        assert!(checklist.mark_succeeded(1, true).is_err());
        let selected = checklist.next_check().expect("highest-priority check");
        assert_eq!(selected, 0);
        assert_eq!(checklist.pairs[0].state, CandidatePairState::InProgress);
        checklist.mark_succeeded(selected, true).expect("valid pair index");
        assert_eq!(checklist.state(), ChecklistState::Completed);
    }
}
