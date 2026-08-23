//! RFC 6184 packetization-mode 1 for Annex-B H.264 access units.

use crate::{RtpPacket, WebRtcError};

const FU_A_TYPE: u8 = 28;
const ANNEX_B_START_CODE: [u8; 4] = [0, 0, 0, 1];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct H264PacketizationConfig {
    pub payload_type: u8,
    pub first_sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub max_payload_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum H264Depacketization {
    Pending,
    AccessUnit(Vec<u8>),
    Dropped,
}

pub fn packetize_h264_access_unit(
    annex_b: &[u8],
    config: H264PacketizationConfig,
) -> Result<Vec<RtpPacket>, WebRtcError> {
    if config.payload_type > 127 || config.max_payload_bytes < 3 {
        return Err(WebRtcError::InvalidPacket(
            "H.264 RTP packetization configuration is invalid",
        ));
    }
    let nal_units = annex_b_nal_units(annex_b)?;
    let mut payloads = Vec::new();
    for nal in nal_units {
        let nal_type = nal[0] & 0x1f;
        if nal[0] & 0x80 != 0 || !(1..=23).contains(&nal_type) {
            return Err(WebRtcError::InvalidPacket(
                "Annex-B access unit contains an unsupported NAL type",
            ));
        }
        if nal.len() <= config.max_payload_bytes {
            payloads.push(nal.to_vec());
            continue;
        }
        if nal.len() == 1 {
            return Err(WebRtcError::InvalidPacket("H.264 NAL has no payload"));
        }
        let fragment_bytes = config.max_payload_bytes - 2;
        let fragments = nal[1..].chunks(fragment_bytes);
        let fragment_count = fragments.len();
        for (index, fragment) in fragments.enumerate() {
            let mut payload = Vec::with_capacity(fragment.len() + 2);
            payload.push((nal[0] & 0xe0) | FU_A_TYPE);
            payload.push(
                (if index == 0 { 0x80 } else { 0 })
                    | (if index + 1 == fragment_count { 0x40 } else { 0 })
                    | nal_type,
            );
            payload.extend_from_slice(fragment);
            payloads.push(payload);
        }
    }
    if payloads.is_empty() || payloads.len() > u16::MAX as usize + 1 {
        return Err(WebRtcError::InvalidPacket(
            "H.264 access unit produced no bounded RTP packets",
        ));
    }
    let last = payloads.len() - 1;
    Ok(payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| RtpPacket {
            marker: index == last,
            payload_type: config.payload_type,
            sequence_number: config.first_sequence_number.wrapping_add(index as u16),
            timestamp: config.timestamp,
            ssrc: config.ssrc,
            payload,
        })
        .collect())
}

pub struct H264Depacketizer {
    payload_type: u8,
    ssrc: u32,
    max_access_unit_bytes: usize,
    expected_sequence: Option<u16>,
    timestamp: Option<u32>,
    access_unit: Vec<u8>,
    fragmented_header: Option<u8>,
}

impl H264Depacketizer {
    pub fn new(
        payload_type: u8,
        ssrc: u32,
        max_access_unit_bytes: usize,
    ) -> Result<Self, WebRtcError> {
        if payload_type > 127 || max_access_unit_bytes == 0 {
            return Err(WebRtcError::InvalidPacket(
                "H.264 depacketizer configuration is invalid",
            ));
        }
        Ok(Self {
            payload_type,
            ssrc,
            max_access_unit_bytes,
            expected_sequence: None,
            timestamp: None,
            access_unit: Vec::new(),
            fragmented_header: None,
        })
    }

    pub fn push(&mut self, packet: RtpPacket) -> Result<H264Depacketization, WebRtcError> {
        if packet.payload_type != self.payload_type || packet.ssrc != self.ssrc {
            return Err(WebRtcError::InvalidPacket(
                "RTP packet does not match H.264 depacketizer",
            ));
        }
        if self
            .expected_sequence
            .is_some_and(|expected| packet.sequence_number != expected)
        {
            self.reset();
            self.expected_sequence = Some(packet.sequence_number.wrapping_add(1));
            return Ok(H264Depacketization::Dropped);
        }
        self.expected_sequence = Some(packet.sequence_number.wrapping_add(1));
        if self
            .timestamp
            .is_some_and(|timestamp| timestamp != packet.timestamp)
        {
            self.reset_frame();
            self.timestamp = Some(packet.timestamp);
            return Ok(H264Depacketization::Dropped);
        }
        self.timestamp.get_or_insert(packet.timestamp);
        let Some(&nal_header) = packet.payload.first() else {
            self.reset_frame();
            return Err(WebRtcError::InvalidPacket("empty H.264 RTP payload"));
        };
        if nal_header & 0x80 != 0 {
            self.reset_frame();
            return Err(WebRtcError::InvalidPacket(
                "H.264 forbidden-zero bit is set",
            ));
        }
        match nal_header & 0x1f {
            1..=23 => {
                if self.fragmented_header.is_some() {
                    self.reset_frame();
                    return Ok(H264Depacketization::Dropped);
                }
                self.append(&ANNEX_B_START_CODE)?;
                self.append(&packet.payload)?;
            }
            FU_A_TYPE => self.push_fu_a(&packet.payload, packet.marker)?,
            _ => {
                self.reset_frame();
                return Err(WebRtcError::Unsupported(
                    "H.264 RTP aggregation or interleaved packet",
                ));
            }
        }
        if packet.marker {
            if self.fragmented_header.is_some() {
                self.reset_frame();
                return Ok(H264Depacketization::Dropped);
            }
            let access_unit = std::mem::take(&mut self.access_unit);
            self.timestamp = None;
            return Ok(H264Depacketization::AccessUnit(access_unit));
        }
        Ok(H264Depacketization::Pending)
    }

    fn push_fu_a(&mut self, payload: &[u8], marker: bool) -> Result<(), WebRtcError> {
        if payload.len() < 3 {
            self.reset_frame();
            return Err(WebRtcError::InvalidPacket("truncated H.264 FU-A payload"));
        }
        let indicator = payload[0];
        let header = payload[1];
        let start = header & 0x80 != 0;
        let end = header & 0x40 != 0;
        let reserved = header & 0x20 != 0;
        let nal_type = header & 0x1f;
        if reserved || start && end || !(1..=23).contains(&nal_type) || marker && !end {
            self.reset_frame();
            return Err(WebRtcError::InvalidPacket("invalid H.264 FU-A header"));
        }
        let reconstructed = (indicator & 0xe0) | nal_type;
        if start {
            if self.fragmented_header.is_some() {
                self.reset_frame();
                return Err(WebRtcError::InvalidPacket("nested H.264 FU-A start"));
            }
            self.append(&ANNEX_B_START_CODE)?;
            self.append(&[reconstructed])?;
            self.fragmented_header = Some(reconstructed);
        } else if self.fragmented_header != Some(reconstructed) {
            self.reset_frame();
            return Err(WebRtcError::InvalidPacket(
                "H.264 FU-A continuation has no matching start",
            ));
        }
        self.append(&payload[2..])?;
        if end {
            self.fragmented_header = None;
        }
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), WebRtcError> {
        if self.access_unit.len().saturating_add(bytes.len()) > self.max_access_unit_bytes {
            self.reset_frame();
            return Err(WebRtcError::InvalidPacket(
                "H.264 access unit exceeds depacketizer cap",
            ));
        }
        self.access_unit.extend_from_slice(bytes);
        Ok(())
    }

    fn reset_frame(&mut self) {
        self.timestamp = None;
        self.access_unit.clear();
        self.fragmented_header = None;
    }

    fn reset(&mut self) {
        self.expected_sequence = None;
        self.reset_frame();
    }
}

fn annex_b_nal_units(access_unit: &[u8]) -> Result<Vec<&[u8]>, WebRtcError> {
    let Some((first, _)) = find_start_code(access_unit, 0) else {
        return Err(WebRtcError::InvalidPacket(
            "H.264 access unit has no Annex-B start code",
        ));
    };
    if access_unit[..first].iter().any(|byte| *byte != 0) {
        return Err(WebRtcError::InvalidPacket(
            "nonzero bytes precede H.264 Annex-B start code",
        ));
    }
    let mut units = Vec::new();
    let mut cursor = first;
    loop {
        let (_, start_len) = find_start_code(access_unit, cursor).ok_or(
            WebRtcError::InvalidPacket("missing H.264 Annex-B start code"),
        )?;
        let nal_start = cursor + start_len;
        let next = find_start_code(access_unit, nal_start).map(|(index, _)| index);
        let nal_end = next.unwrap_or(access_unit.len());
        if nal_start >= nal_end {
            return Err(WebRtcError::InvalidPacket("empty H.264 Annex-B NAL"));
        }
        units.push(&access_unit[nal_start..nal_end]);
        match next {
            Some(next) => cursor = next,
            None => break,
        }
    }
    Ok(units)
}

fn find_start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    for index in from..bytes.len() {
        if bytes.get(index..index + 4) == Some(&[0, 0, 0, 1]) {
            return Some((index, 4));
        }
        if bytes.get(index..index + 3) == Some(&[0, 0, 1]) {
            return Some((index, 3));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalized(units: &[&[u8]]) -> Vec<u8> {
        let mut output = Vec::new();
        for unit in units {
            output.extend_from_slice(&ANNEX_B_START_CODE);
            output.extend_from_slice(unit);
        }
        output
    }

    #[test]
    fn single_nal_and_fu_a_packets_reconstruct_one_access_unit() {
        let small = [0x67, 1, 2, 3];
        let mut large = vec![0x65];
        large.extend((0..3_000).map(|index| index as u8));
        let access_unit = normalized(&[&small, &large]);
        let packets = packetize_h264_access_unit(
            &access_unit,
            H264PacketizationConfig {
                payload_type: 102,
                first_sequence_number: u16::MAX - 1,
                timestamp: 90_000,
                ssrc: 7,
                max_payload_bytes: 1_200,
            },
        )
        .unwrap();
        assert_eq!(packets.len(), 4);
        assert_eq!(packets[0].payload, small);
        assert_eq!(packets[1].payload[0] & 0x1f, FU_A_TYPE);
        assert_eq!(packets[1].payload[1] & 0x80, 0x80);
        assert_eq!(packets[3].payload[1] & 0x40, 0x40);
        assert_eq!(packets[3].sequence_number, 1);
        assert!(packets.iter().all(|packet| packet.payload.len() <= 1_200));
        assert!(packets[..3].iter().all(|packet| !packet.marker));
        assert!(packets[3].marker);

        let mut depacketizer = H264Depacketizer::new(102, 7, 10_000).unwrap();
        let mut recovered = None;
        for packet in packets {
            if let H264Depacketization::AccessUnit(access_unit) = depacketizer.push(packet).unwrap()
            {
                recovered = Some(access_unit);
            }
        }
        assert_eq!(recovered.unwrap(), access_unit);
    }

    #[test]
    fn sequence_gap_drops_damage_and_next_marker_recovers() {
        let mut depacketizer = H264Depacketizer::new(102, 7, 1_024).unwrap();
        let first = RtpPacket {
            marker: false,
            payload_type: 102,
            sequence_number: 10,
            timestamp: 1,
            ssrc: 7,
            payload: vec![0x7c, 0x85, 1, 2],
        };
        assert_eq!(
            depacketizer.push(first).unwrap(),
            H264Depacketization::Pending
        );
        let gap = RtpPacket {
            marker: true,
            payload_type: 102,
            sequence_number: 12,
            timestamp: 1,
            ssrc: 7,
            payload: vec![0x7c, 0x45, 3, 4],
        };
        assert_eq!(
            depacketizer.push(gap).unwrap(),
            H264Depacketization::Dropped
        );
        let recovery = RtpPacket {
            marker: true,
            payload_type: 102,
            sequence_number: 13,
            timestamp: 2,
            ssrc: 7,
            payload: vec![0x65, 9, 8, 7],
        };
        assert_eq!(
            depacketizer.push(recovery).unwrap(),
            H264Depacketization::AccessUnit(normalized(&[&[0x65, 9, 8, 7]]))
        );
    }

    #[test]
    fn malformed_fu_and_oversized_access_unit_are_rejected() {
        let malformed = RtpPacket {
            marker: true,
            payload_type: 102,
            sequence_number: 1,
            timestamp: 1,
            ssrc: 7,
            payload: vec![0x7c, 0xc5, 1],
        };
        let mut depacketizer = H264Depacketizer::new(102, 7, 4).unwrap();
        assert!(depacketizer.push(malformed).is_err());
        let oversized = RtpPacket {
            marker: true,
            payload_type: 102,
            sequence_number: 2,
            timestamp: 2,
            ssrc: 7,
            payload: vec![0x65, 1, 2, 3, 4],
        };
        assert!(depacketizer.push(oversized).is_err());
        assert!(
            packetize_h264_access_unit(
                &[1, 2, 3],
                H264PacketizationConfig {
                    payload_type: 102,
                    first_sequence_number: 1,
                    timestamp: 1,
                    ssrc: 7,
                    max_payload_bytes: 1_200,
                }
            )
            .is_err()
        );
    }
}
