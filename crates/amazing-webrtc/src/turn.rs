//! Authenticated IPv4 UDP TURN allocation and relay probe.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Duration;

use crate::{
    CandidatePairState, ChecklistState, IceCandidate, IceCandidateKind, IceChecklist,
    STUN_MAGIC_COOKIE, TransactionId, WebRtcError, append_stun_attribute, candidate_priority,
    crc32, finish_authenticated_stun, set_stun_body_length, stun_attribute_ranges, stun_header,
    verify_fingerprint, verify_message_integrity,
};

const ALLOCATE_REQUEST: u16 = 0x0003;
const ALLOCATE_SUCCESS: u16 = 0x0103;
const ALLOCATE_ERROR: u16 = 0x0113;
const REFRESH_REQUEST: u16 = 0x0004;
const REFRESH_SUCCESS: u16 = 0x0104;
const SEND_INDICATION: u16 = 0x0016;
const DATA_INDICATION: u16 = 0x0017;
const CREATE_PERMISSION_REQUEST: u16 = 0x0008;
const CREATE_PERMISSION_SUCCESS: u16 = 0x0108;

const USERNAME: u16 = 0x0006;
const MESSAGE_INTEGRITY: u16 = 0x0008;
const ERROR_CODE: u16 = 0x0009;
const LIFETIME: u16 = 0x000d;
const XOR_PEER_ADDRESS: u16 = 0x0012;
const DATA: u16 = 0x0013;
const REALM: u16 = 0x0014;
const NONCE: u16 = 0x0015;
const XOR_RELAYED_ADDRESS: u16 = 0x0016;
const REQUESTED_TRANSPORT: u16 = 0x0019;
const XOR_MAPPED_ADDRESS: u16 = 0x0020;
const FINGERPRINT: u16 = 0x8028;

const USER: &str = "amazing-user";
const REALM_VALUE: &str = "turn.amazing.test";
const PASSWORD: &str = "AmazingTurnPassword";
const NONCE_VALUE: &str = "obMatJos2AAAAamazing-nonce";
const LIFETIME_SECONDS: u32 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnProbeReport {
    pub challenge_code: u16,
    pub relayed_address: SocketAddrV4,
    pub mapped_address: SocketAddrV4,
    pub lifetime_seconds: u32,
    pub permission_created: bool,
    pub client_to_peer_bytes: usize,
    pub peer_to_client_bytes: usize,
    pub allocation_deleted: bool,
    pub relay_candidate: IceCandidate,
    pub relay_nominated: bool,
}

#[derive(Debug)]
struct TurnMessage {
    kind: u16,
    transaction_id: TransactionId,
    attributes: Vec<(u16, Vec<u8>)>,
}

impl TurnMessage {
    fn parse(packet: &[u8]) -> Result<Self, WebRtcError> {
        if packet.len() < 20 || packet[0] & 0xc0 != 0 {
            return Err(WebRtcError::InvalidPacket("invalid TURN/STUN header"));
        }
        let declared = u16::from_be_bytes([packet[2], packet[3]]) as usize;
        if !declared.is_multiple_of(4)
            || packet.len() != 20 + declared
            || u32::from_be_bytes(packet[4..8].try_into().expect("TURN cookie bounds"))
                != STUN_MAGIC_COOKIE
        {
            return Err(WebRtcError::InvalidPacket(
                "TURN/STUN length or cookie mismatch",
            ));
        }
        let ranges = stun_attribute_ranges(packet)?;
        if ranges.iter().any(|(kind, _, _, _)| *kind == FINGERPRINT) && !verify_fingerprint(packet)?
        {
            return Err(WebRtcError::InvalidPacket("TURN/STUN fingerprint mismatch"));
        }
        let mut transaction = [0; 12];
        transaction.copy_from_slice(&packet[8..20]);
        let attributes = ranges
            .into_iter()
            .map(|(kind, _, start, end)| (kind, packet[start..end].to_vec()))
            .collect();
        Ok(Self {
            kind: u16::from_be_bytes([packet[0], packet[1]]),
            transaction_id: TransactionId(transaction),
            attributes,
        })
    }

    fn attribute(&self, kind: u16) -> Result<Option<&[u8]>, WebRtcError> {
        let mut values = self
            .attributes
            .iter()
            .filter(|(candidate, _)| *candidate == kind)
            .map(|(_, value)| value.as_slice());
        let first = values.next();
        if values.next().is_some() {
            return Err(WebRtcError::InvalidPacket("duplicate TURN attribute"));
        }
        Ok(first)
    }

    fn required(&self, kind: u16) -> Result<&[u8], WebRtcError> {
        self.attribute(kind)?.ok_or(WebRtcError::InvalidPacket(
            "required TURN attribute is missing",
        ))
    }
}

pub fn run_turn_udp_probe() -> Result<TurnProbeReport, WebRtcError> {
    let server_control = bind_udp("cannot bind TURN control socket")?;
    let relay = bind_udp("cannot bind TURN relay socket")?;
    let client = bind_udp("cannot bind TURN client socket")?;
    let peer = bind_udp("cannot bind TURN peer socket")?;
    for socket in [&server_control, &relay, &client, &peer] {
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| WebRtcError::InvalidPacket("cannot configure TURN timeout"))?;
    }
    let server_address = ipv4_address(&server_control)?;
    let relay_address = ipv4_address(&relay)?;
    let client_address = ipv4_address(&client)?;
    let peer_address = ipv4_address(&peer)?;
    client
        .connect(server_address)
        .map_err(|_| WebRtcError::InvalidPacket("cannot connect TURN client"))?;

    let peer_thread = std::thread::spawn(move || -> Result<usize, WebRtcError> {
        let mut packet = [0; 1_024];
        let (length, source) = peer
            .recv_from(&mut packet)
            .map_err(|_| WebRtcError::InvalidPacket("TURN peer receive failed"))?;
        if &packet[..length] != b"through-turn" {
            return Err(WebRtcError::InvalidPacket(
                "TURN peer received wrong payload",
            ));
        }
        peer.send_to(b"relay-return", source)
            .map_err(|_| WebRtcError::InvalidPacket("TURN peer reply failed"))?;
        Ok(length)
    });

    let server_thread = std::thread::spawn(move || {
        run_loopback_turn_server(
            server_control,
            relay,
            relay_address,
            client_address,
            peer_address,
        )
    });

    let initial_transaction = TransactionId(*b"turnalloc001");
    let mut initial = stun_header(ALLOCATE_REQUEST, initial_transaction);
    append_stun_attribute(&mut initial, REQUESTED_TRANSPORT, &[17, 0, 0, 0])?;
    let initial = finish_plain_stun(initial)?;
    client
        .send(&initial)
        .map_err(|_| WebRtcError::InvalidPacket("TURN initial Allocate send failed"))?;
    let challenge_wire = receive_packet(&client, "TURN Allocate challenge timed out")?;
    let challenge = TurnMessage::parse(&challenge_wire)?;
    if challenge.kind != ALLOCATE_ERROR || challenge.transaction_id != initial_transaction {
        return Err(WebRtcError::InvalidPacket(
            "unexpected TURN Allocate challenge",
        ));
    }
    let challenge_code = parse_error_code(challenge.required(ERROR_CODE)?)?;
    let realm = parse_text(challenge.required(REALM)?)?;
    let nonce = parse_text(challenge.required(NONCE)?)?;
    if challenge_code != 401 || realm != REALM_VALUE || nonce != NONCE_VALUE {
        return Err(WebRtcError::InvalidPacket(
            "invalid TURN long-term challenge",
        ));
    }

    let key = long_term_key(USER, REALM_VALUE, PASSWORD);
    let allocate_transaction = TransactionId(*b"turnalloc002");
    let mut allocate = stun_header(ALLOCATE_REQUEST, allocate_transaction);
    append_stun_attribute(&mut allocate, USERNAME, USER.as_bytes())?;
    append_stun_attribute(&mut allocate, REALM, REALM_VALUE.as_bytes())?;
    append_stun_attribute(&mut allocate, NONCE, NONCE_VALUE.as_bytes())?;
    append_stun_attribute(&mut allocate, REQUESTED_TRANSPORT, &[17, 0, 0, 0])?;
    append_stun_attribute(&mut allocate, LIFETIME, &LIFETIME_SECONDS.to_be_bytes())?;
    let allocate = finish_authenticated_stun(allocate, &key)?;
    client
        .send(&allocate)
        .map_err(|_| WebRtcError::InvalidPacket("TURN authenticated Allocate send failed"))?;
    let success_wire = receive_packet(&client, "TURN Allocate success timed out")?;
    if !verify_message_integrity(&success_wire, &key)? {
        return Err(WebRtcError::InvalidPacket("TURN Allocate integrity failed"));
    }
    let success = TurnMessage::parse(&success_wire)?;
    if success.kind != ALLOCATE_SUCCESS || success.transaction_id != allocate_transaction {
        return Err(WebRtcError::InvalidPacket(
            "unexpected TURN Allocate success",
        ));
    }
    let allocated_relay = decode_xor_address(success.required(XOR_RELAYED_ADDRESS)?)?;
    let mapped_address = decode_xor_address(success.required(XOR_MAPPED_ADDRESS)?)?;
    let lifetime_seconds = parse_u32(success.required(LIFETIME)?)?;

    let permission_transaction = TransactionId(*b"turnperm0001");
    let permission = authenticated_request(
        CREATE_PERMISSION_REQUEST,
        permission_transaction,
        &key,
        &[(XOR_PEER_ADDRESS, encode_xor_address(peer_address).to_vec())],
    )?;
    client
        .send(&permission)
        .map_err(|_| WebRtcError::InvalidPacket("TURN permission send failed"))?;
    let permission_wire = receive_packet(&client, "TURN permission success timed out")?;
    if !verify_message_integrity(&permission_wire, &key)? {
        return Err(WebRtcError::InvalidPacket(
            "TURN permission integrity failed",
        ));
    }
    let permission_response = TurnMessage::parse(&permission_wire)?;
    if permission_response.kind != CREATE_PERMISSION_SUCCESS
        || permission_response.transaction_id != permission_transaction
    {
        return Err(WebRtcError::InvalidPacket(
            "unexpected TURN permission response",
        ));
    }

    let mut send = stun_header(SEND_INDICATION, TransactionId(*b"turnsend0001"));
    append_stun_attribute(
        &mut send,
        XOR_PEER_ADDRESS,
        &encode_xor_address(peer_address),
    )?;
    append_stun_attribute(&mut send, DATA, b"through-turn")?;
    let send = finish_plain_stun(send)?;
    client
        .send(&send)
        .map_err(|_| WebRtcError::InvalidPacket("TURN Send indication failed"))?;
    let data_wire = receive_packet(&client, "TURN Data indication timed out")?;
    let data = TurnMessage::parse(&data_wire)?;
    if data.kind != DATA_INDICATION
        || decode_xor_address(data.required(XOR_PEER_ADDRESS)?)? != peer_address
        || data.required(DATA)? != b"relay-return"
    {
        return Err(WebRtcError::InvalidPacket("invalid TURN Data indication"));
    }

    let refresh_transaction = TransactionId(*b"turnrefr0001");
    let refresh = authenticated_request(
        REFRESH_REQUEST,
        refresh_transaction,
        &key,
        &[(LIFETIME, 0u32.to_be_bytes().to_vec())],
    )?;
    client
        .send(&refresh)
        .map_err(|_| WebRtcError::InvalidPacket("TURN Refresh send failed"))?;
    let refresh_wire = receive_packet(&client, "TURN Refresh success timed out")?;
    if !verify_message_integrity(&refresh_wire, &key)? {
        return Err(WebRtcError::InvalidPacket("TURN Refresh integrity failed"));
    }
    let refresh_response = TurnMessage::parse(&refresh_wire)?;
    let allocation_deleted = refresh_response.kind == REFRESH_SUCCESS
        && refresh_response.transaction_id == refresh_transaction
        && parse_u32(refresh_response.required(LIFETIME)?)? == 0;

    let client_to_peer_bytes = peer_thread
        .join()
        .map_err(|_| WebRtcError::InvalidPacket("TURN peer thread panicked"))??;
    server_thread
        .join()
        .map_err(|_| WebRtcError::InvalidPacket("TURN server thread panicked"))??;

    let relay_candidate = IceCandidate {
        foundation: "turnudp".into(),
        component: 1,
        priority: candidate_priority(IceCandidateKind::Relay, u16::MAX, 1)?,
        address: allocated_relay,
        kind: IceCandidateKind::Relay,
        related_address: Some(mapped_address),
    };
    let remote_candidate = IceCandidate {
        foundation: "turn-peer".into(),
        component: 1,
        priority: candidate_priority(IceCandidateKind::Host, u16::MAX, 1)?,
        address: peer_address,
        kind: IceCandidateKind::Host,
        related_address: None,
    };
    let mut checklist = IceChecklist::new(
        std::slice::from_ref(&relay_candidate),
        &[remote_candidate],
        true,
    );
    let pair = checklist
        .next_check()
        .ok_or(WebRtcError::InvalidPacket("TURN relay ICE pair is missing"))?;
    if checklist.pairs[pair].state != CandidatePairState::InProgress {
        return Err(WebRtcError::InvalidPacket(
            "TURN relay ICE pair did not start",
        ));
    }
    checklist.mark_succeeded(pair, true)?;
    let relay_nominated = checklist.state() == ChecklistState::Completed;

    Ok(TurnProbeReport {
        challenge_code,
        relayed_address: allocated_relay,
        mapped_address,
        lifetime_seconds,
        permission_created: true,
        client_to_peer_bytes,
        peer_to_client_bytes: b"relay-return".len(),
        allocation_deleted,
        relay_candidate,
        relay_nominated,
    })
}

fn run_loopback_turn_server(
    control: UdpSocket,
    relay: UdpSocket,
    relay_address: SocketAddrV4,
    expected_client: SocketAddrV4,
    expected_peer: SocketAddrV4,
) -> Result<(), WebRtcError> {
    let mut packet = [0; 65_535];
    let (length, client_source) = control
        .recv_from(&mut packet)
        .map_err(|_| WebRtcError::InvalidPacket("TURN server initial receive failed"))?;
    if client_source != SocketAddr::V4(expected_client) {
        return Err(WebRtcError::InvalidPacket("TURN client address changed"));
    }
    let initial = TurnMessage::parse(&packet[..length])?;
    if initial.kind != ALLOCATE_REQUEST
        || initial.required(REQUESTED_TRANSPORT)? != [17, 0, 0, 0]
        || initial.attribute(MESSAGE_INTEGRITY)?.is_some()
    {
        return Err(WebRtcError::InvalidPacket("invalid initial TURN Allocate"));
    }
    let challenge = encode_challenge(initial.transaction_id)?;
    control
        .send_to(&challenge, client_source)
        .map_err(|_| WebRtcError::InvalidPacket("TURN challenge send failed"))?;

    let (length, source) = control
        .recv_from(&mut packet)
        .map_err(|_| WebRtcError::InvalidPacket("TURN authenticated receive failed"))?;
    if source != client_source {
        return Err(WebRtcError::InvalidPacket(
            "TURN allocation 5-tuple changed",
        ));
    }
    let allocate_wire = &packet[..length];
    let allocate = TurnMessage::parse(allocate_wire)?;
    let key = validate_authenticated(&allocate, allocate_wire, ALLOCATE_REQUEST)?;
    if allocate.required(REQUESTED_TRANSPORT)? != [17, 0, 0, 0] {
        return Err(WebRtcError::InvalidPacket(
            "TURN requested transport is not UDP",
        ));
    }
    let requested_lifetime = allocate
        .attribute(LIFETIME)?
        .map(parse_u32)
        .transpose()?
        .unwrap_or(LIFETIME_SECONDS)
        .min(3_600);
    let mut success = stun_header(ALLOCATE_SUCCESS, allocate.transaction_id);
    append_stun_attribute(
        &mut success,
        XOR_RELAYED_ADDRESS,
        &encode_xor_address(relay_address),
    )?;
    append_stun_attribute(
        &mut success,
        XOR_MAPPED_ADDRESS,
        &encode_xor_address(expected_client),
    )?;
    append_stun_attribute(&mut success, LIFETIME, &requested_lifetime.to_be_bytes())?;
    let success = finish_authenticated_stun(success, &key)?;
    control
        .send_to(&success, client_source)
        .map_err(|_| WebRtcError::InvalidPacket("TURN Allocate success send failed"))?;

    let (length, source) = control
        .recv_from(&mut packet)
        .map_err(|_| WebRtcError::InvalidPacket("TURN permission receive failed"))?;
    if source != client_source {
        return Err(WebRtcError::InvalidPacket(
            "TURN permission 5-tuple changed",
        ));
    }
    let permission_wire = &packet[..length];
    let permission = TurnMessage::parse(permission_wire)?;
    validate_authenticated(&permission, permission_wire, CREATE_PERMISSION_REQUEST)?;
    if decode_xor_address(permission.required(XOR_PEER_ADDRESS)?)? != expected_peer {
        return Err(WebRtcError::InvalidPacket("TURN permission peer mismatch"));
    }
    let permission_success = finish_authenticated_stun(
        stun_header(CREATE_PERMISSION_SUCCESS, permission.transaction_id),
        &key,
    )?;
    control
        .send_to(&permission_success, client_source)
        .map_err(|_| WebRtcError::InvalidPacket("TURN permission success send failed"))?;

    let (length, source) = control
        .recv_from(&mut packet)
        .map_err(|_| WebRtcError::InvalidPacket("TURN Send indication receive failed"))?;
    if source != client_source {
        return Err(WebRtcError::InvalidPacket("TURN Send 5-tuple changed"));
    }
    let send = TurnMessage::parse(&packet[..length])?;
    if send.kind != SEND_INDICATION
        || decode_xor_address(send.required(XOR_PEER_ADDRESS)?)? != expected_peer
    {
        return Err(WebRtcError::InvalidPacket("invalid TURN Send indication"));
    }
    relay
        .send_to(send.required(DATA)?, expected_peer)
        .map_err(|_| WebRtcError::InvalidPacket("TURN relay outbound send failed"))?;

    let (length, peer_source) = relay
        .recv_from(&mut packet)
        .map_err(|_| WebRtcError::InvalidPacket("TURN relay return receive failed"))?;
    if peer_source != SocketAddr::V4(expected_peer) {
        return Err(WebRtcError::InvalidPacket(
            "TURN relay return peer mismatch",
        ));
    }
    let mut data = stun_header(DATA_INDICATION, TransactionId(*b"turndata0001"));
    append_stun_attribute(
        &mut data,
        XOR_PEER_ADDRESS,
        &encode_xor_address(expected_peer),
    )?;
    append_stun_attribute(&mut data, DATA, &packet[..length])?;
    let data = finish_plain_stun(data)?;
    control
        .send_to(&data, client_source)
        .map_err(|_| WebRtcError::InvalidPacket("TURN Data indication send failed"))?;

    let (length, source) = control
        .recv_from(&mut packet)
        .map_err(|_| WebRtcError::InvalidPacket("TURN Refresh receive failed"))?;
    if source != client_source {
        return Err(WebRtcError::InvalidPacket("TURN Refresh 5-tuple changed"));
    }
    let refresh_wire = &packet[..length];
    let refresh = TurnMessage::parse(refresh_wire)?;
    validate_authenticated(&refresh, refresh_wire, REFRESH_REQUEST)?;
    if parse_u32(refresh.required(LIFETIME)?)? != 0 {
        return Err(WebRtcError::InvalidPacket(
            "TURN probe Refresh did not delete",
        ));
    }
    let mut refresh_success = stun_header(REFRESH_SUCCESS, refresh.transaction_id);
    append_stun_attribute(&mut refresh_success, LIFETIME, &0u32.to_be_bytes())?;
    let refresh_success = finish_authenticated_stun(refresh_success, &key)?;
    control
        .send_to(&refresh_success, client_source)
        .map_err(|_| WebRtcError::InvalidPacket("TURN Refresh success send failed"))?;
    Ok(())
}

fn authenticated_request(
    kind: u16,
    transaction_id: TransactionId,
    key: &[u8; 16],
    extra: &[(u16, Vec<u8>)],
) -> Result<Vec<u8>, WebRtcError> {
    let mut packet = stun_header(kind, transaction_id);
    append_stun_attribute(&mut packet, USERNAME, USER.as_bytes())?;
    append_stun_attribute(&mut packet, REALM, REALM_VALUE.as_bytes())?;
    append_stun_attribute(&mut packet, NONCE, NONCE_VALUE.as_bytes())?;
    for (attribute, value) in extra {
        append_stun_attribute(&mut packet, *attribute, value)?;
    }
    finish_authenticated_stun(packet, key)
}

fn validate_authenticated(
    message: &TurnMessage,
    wire: &[u8],
    expected_kind: u16,
) -> Result<[u8; 16], WebRtcError> {
    if message.kind != expected_kind
        || parse_text(message.required(USERNAME)?)? != USER
        || parse_text(message.required(REALM)?)? != REALM_VALUE
        || parse_text(message.required(NONCE)?)? != NONCE_VALUE
    {
        return Err(WebRtcError::InvalidPacket(
            "TURN long-term credential mismatch",
        ));
    }
    let key = long_term_key(USER, REALM_VALUE, PASSWORD);
    if !verify_message_integrity(wire, &key)? {
        return Err(WebRtcError::InvalidPacket(
            "TURN MESSAGE-INTEGRITY mismatch",
        ));
    }
    Ok(key)
}

fn encode_challenge(transaction_id: TransactionId) -> Result<Vec<u8>, WebRtcError> {
    let mut packet = stun_header(ALLOCATE_ERROR, transaction_id);
    let mut error = vec![0, 0, 4, 1];
    error.extend_from_slice(b"Unauthorized");
    append_stun_attribute(&mut packet, ERROR_CODE, &error)?;
    append_stun_attribute(&mut packet, REALM, REALM_VALUE.as_bytes())?;
    append_stun_attribute(&mut packet, NONCE, NONCE_VALUE.as_bytes())?;
    finish_plain_stun(packet)
}

fn finish_plain_stun(mut packet: Vec<u8>) -> Result<Vec<u8>, WebRtcError> {
    let fingerprint_end = packet
        .len()
        .checked_add(8)
        .ok_or(WebRtcError::InvalidPacket(
            "TURN fingerprint length overflow",
        ))?;
    set_stun_body_length(&mut packet, fingerprint_end)?;
    let fingerprint = crc32(&packet) ^ 0x5354_554e;
    append_stun_attribute(&mut packet, FINGERPRINT, &fingerprint.to_be_bytes())?;
    Ok(packet)
}

fn encode_xor_address(address: SocketAddrV4) -> [u8; 8] {
    let mut value = [0; 8];
    value[1] = 1;
    value[2..4].copy_from_slice(&(address.port() ^ (STUN_MAGIC_COOKIE >> 16) as u16).to_be_bytes());
    value[4..8].copy_from_slice(
        &(u32::from_be_bytes(address.ip().octets()) ^ STUN_MAGIC_COOKIE).to_be_bytes(),
    );
    value
}

fn decode_xor_address(value: &[u8]) -> Result<SocketAddrV4, WebRtcError> {
    if value.len() != 8 || value[0] != 0 || value[1] != 1 {
        return Err(WebRtcError::Unsupported(
            "TURN probe supports IPv4 addresses",
        ));
    }
    let port = u16::from_be_bytes([value[2], value[3]]) ^ (STUN_MAGIC_COOKIE >> 16) as u16;
    let encoded = u32::from_be_bytes(value[4..8].try_into().expect("TURN IPv4 bounds"));
    Ok(SocketAddrV4::new(
        Ipv4Addr::from(encoded ^ STUN_MAGIC_COOKIE),
        port,
    ))
}

fn parse_error_code(value: &[u8]) -> Result<u16, WebRtcError> {
    if value.len() < 4 || value[0] != 0 || value[1] != 0 || value[2] > 6 || value[3] > 99 {
        return Err(WebRtcError::InvalidPacket("invalid TURN ERROR-CODE"));
    }
    Ok(value[2] as u16 * 100 + value[3] as u16)
}

fn parse_u32(value: &[u8]) -> Result<u32, WebRtcError> {
    if value.len() != 4 {
        return Err(WebRtcError::InvalidPacket(
            "TURN integer attribute length mismatch",
        ));
    }
    Ok(u32::from_be_bytes(
        value.try_into().expect("TURN integer bounds"),
    ))
}

fn parse_text(value: &[u8]) -> Result<&str, WebRtcError> {
    std::str::from_utf8(value).map_err(|_| WebRtcError::InvalidPacket("TURN text is not UTF-8"))
}

fn long_term_key(username: &str, realm: &str, password: &str) -> [u8; 16] {
    md5(format!("{username}:{realm}:{password}").as_bytes())
}

fn bind_udp(error: &'static str) -> Result<UdpSocket, WebRtcError> {
    UdpSocket::bind("127.0.0.1:0").map_err(|_| WebRtcError::InvalidPacket(error))
}

fn ipv4_address(socket: &UdpSocket) -> Result<SocketAddrV4, WebRtcError> {
    match socket
        .local_addr()
        .map_err(|_| WebRtcError::InvalidPacket("cannot read TURN socket address"))?
    {
        SocketAddr::V4(address) => Ok(address),
        SocketAddr::V6(_) => Err(WebRtcError::Unsupported("TURN probe requires IPv4")),
    }
}

fn receive_packet(socket: &UdpSocket, error: &'static str) -> Result<Vec<u8>, WebRtcError> {
    let mut packet = vec![0; 65_535];
    let length = socket
        .recv(&mut packet)
        .map_err(|_| WebRtcError::InvalidPacket(error))?;
    packet.truncate(length);
    Ok(packet)
}

fn md5(message: &[u8]) -> [u8; 16] {
    const SHIFT: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const TABLE: [u32; 64] = [
        0xd76a_a478,
        0xe8c7_b756,
        0x2420_70db,
        0xc1bd_ceee,
        0xf57c_0faf,
        0x4787_c62a,
        0xa830_4613,
        0xfd46_9501,
        0x6980_98d8,
        0x8b44_f7af,
        0xffff_5bb1,
        0x895c_d7be,
        0x6b90_1122,
        0xfd98_7193,
        0xa679_438e,
        0x49b4_0821,
        0xf61e_2562,
        0xc040_b340,
        0x265e_5a51,
        0xe9b6_c7aa,
        0xd62f_105d,
        0x0244_1453,
        0xd8a1_e681,
        0xe7d3_fbc8,
        0x21e1_cde6,
        0xc337_07d6,
        0xf4d5_0d87,
        0x455a_14ed,
        0xa9e3_e905,
        0xfcef_a3f8,
        0x676f_02d9,
        0x8d2a_4c8a,
        0xfffa_3942,
        0x8771_f681,
        0x6d9d_6122,
        0xfde5_380c,
        0xa4be_ea44,
        0x4bde_cfa9,
        0xf6bb_4b60,
        0xbebf_bc70,
        0x289b_7ec6,
        0xeaa1_27fa,
        0xd4ef_3085,
        0x0488_1d05,
        0xd9d4_d039,
        0xe6db_99e5,
        0x1fa2_7cf8,
        0xc4ac_5665,
        0xf429_2244,
        0x432a_ff97,
        0xab94_23a7,
        0xfc93_a039,
        0x655b_59c3,
        0x8f0c_cc92,
        0xffef_f47d,
        0x8584_5dd1,
        0x6fa8_7e4f,
        0xfe2c_e6e0,
        0xa301_4314,
        0x4e08_11a1,
        0xf753_7e82,
        0xbd3a_f235,
        0x2ad7_d2bb,
        0xeb86_d391,
    ];
    let bit_length = (message.len() as u64).wrapping_mul(8);
    let mut padded = message.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_le_bytes());
    let mut state = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 16];
        for (word, bytes) in words.iter_mut().zip(chunk.chunks_exact(4)) {
            *word = u32::from_le_bytes(bytes.try_into().expect("MD5 word bounds"));
        }
        let [mut a, mut b, mut c, mut d] = state;
        for index in 0..64 {
            let (mix, word) = match index {
                0..=15 => ((b & c) | (!b & d), index),
                16..=31 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let next = a
                .wrapping_add(mix)
                .wrapping_add(TABLE[index])
                .wrapping_add(words[word]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(next.rotate_left(SHIFT[index]));
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    let mut digest = [0; 16];
    for (bytes, word) in digest.chunks_exact_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_term_key_matches_rfc8489_example() {
        assert_eq!(
            long_term_key("user", "realm", "pass"),
            [
                0x84, 0x93, 0xfb, 0xc5, 0x3b, 0xa5, 0x82, 0xfb, 0x4c, 0x04, 0x4c, 0x45, 0x6b, 0xdc,
                0x40, 0xeb,
            ]
        );
    }

    #[test]
    fn authenticated_allocate_permission_relay_and_delete_complete() {
        let report = run_turn_udp_probe().unwrap();
        assert_eq!(report.challenge_code, 401);
        assert_eq!(report.relayed_address.ip(), &Ipv4Addr::LOCALHOST);
        assert_eq!(report.mapped_address.ip(), &Ipv4Addr::LOCALHOST);
        assert_eq!(report.lifetime_seconds, LIFETIME_SECONDS);
        assert!(report.permission_created && report.allocation_deleted);
        assert_eq!(report.relay_candidate.kind, IceCandidateKind::Relay);
        assert_eq!(
            report.relay_candidate.related_address,
            Some(report.mapped_address)
        );
        assert!(report.relay_nominated);
        assert_eq!(report.client_to_peer_bytes, 12);
        assert_eq!(report.peer_to_client_bytes, 12);
    }

    #[test]
    fn turn_parser_rejects_wrong_cookie_and_duplicate_required_attribute() {
        let transaction = TransactionId(*b"turnbad00001");
        let mut packet = stun_header(ALLOCATE_REQUEST, transaction);
        append_stun_attribute(&mut packet, REQUESTED_TRANSPORT, &[17, 0, 0, 0]).unwrap();
        append_stun_attribute(&mut packet, REQUESTED_TRANSPORT, &[17, 0, 0, 0]).unwrap();
        let packet = finish_plain_stun(packet).unwrap();
        let message = TurnMessage::parse(&packet).unwrap();
        assert!(message.attribute(REQUESTED_TRANSPORT).is_err());
        let mut wrong_cookie = packet;
        wrong_cookie[4] ^= 1;
        assert!(TurnMessage::parse(&wrong_cookie).is_err());
    }
}
