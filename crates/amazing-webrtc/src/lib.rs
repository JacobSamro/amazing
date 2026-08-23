//! Phase 9.3 WebRTC transport foundations.
//!
//! STUN and RTP wire details stay behind small parse/encode interfaces. The
//! crate is safe, synchronous, and `std`-only; ICE state machines, DTLS/SRTP,
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
        if attribute_type == 0x0020 {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
