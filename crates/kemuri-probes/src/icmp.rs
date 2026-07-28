use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use kemuri_core::{ProbeKind, SampleOutcome};
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;

use crate::{Probe, ProbeExecutionError, ProbeRound, ResolvedCheck, RoundContext, SampleResult};

const ICMP_ECHO_REQUEST_V4: u8 = 8;
const ICMP_ECHO_REPLY_V4: u8 = 0;
const ICMP_DEST_UNREACH_V4: u8 = 3;
const ICMP_TIME_EXCEEDED_V4: u8 = 11;

const ICMP_ECHO_REQUEST_V6: u8 = 128;
const ICMP_ECHO_REPLY_V6: u8 = 129;
const ICMP_DEST_UNREACH_V6: u8 = 1;
const ICMP_TIME_EXCEEDED_V6: u8 = 3;

const DEFAULT_PAYLOAD_SIZE: usize = 56;
const RECV_BUFFER_SIZE: usize = 2048;
const MAX_RECV_ATTEMPTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressFamily {
    #[default]
    Auto,
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcmpProbeConfig {
    pub address_family: AddressFamily,
    pub payload_size: usize,
    pub source_address: Option<String>,
}

impl Default for IcmpProbeConfig {
    fn default() -> Self {
        Self {
            address_family: AddressFamily::Auto,
            payload_size: DEFAULT_PAYLOAD_SIZE,
            source_address: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketMethod {
    Dgram,
    Raw,
}

impl std::fmt::Display for SocketMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketMethod::Dgram => write!(f, "dgram"),
            SocketMethod::Raw => write!(f, "raw"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IcmpCapability {
    pub ipv4: Option<SocketMethod>,
    pub ipv6: Option<SocketMethod>,
}

impl IcmpCapability {
    pub fn is_available(&self) -> bool {
        self.ipv4.is_some() || self.ipv6.is_some()
    }
}

pub fn check_icmp_capability() -> IcmpCapability {
    let ipv4 = try_create_socket_type(Domain::IPV4, Protocol::ICMPV4);
    let ipv6 = try_create_socket_type(Domain::IPV6, Protocol::ICMPV6);
    IcmpCapability { ipv4, ipv6 }
}

fn try_create_socket_type(domain: Domain, protocol: Protocol) -> Option<SocketMethod> {
    if Socket::new(domain, Type::DGRAM, Some(protocol)).is_ok() {
        return Some(SocketMethod::Dgram);
    }
    if Socket::new(domain, Type::RAW, Some(protocol)).is_ok() {
        return Some(SocketMethod::Raw);
    }
    None
}

pub struct IcmpProbe {
    config: IcmpProbeConfig,
}

impl IcmpProbe {
    pub fn new(config: IcmpProbeConfig) -> Self {
        Self { config }
    }

    async fn resolve_host(&self, host: &str) -> Result<(IpAddr, bool), ProbeExecutionError> {
        let lookup_addr = format!("{}:0", host);
        let addrs = tokio::net::lookup_host(&lookup_addr)
            .await
            .map_err(|e| ProbeExecutionError::Dns(e.to_string()))?;

        for socket_addr in addrs {
            let ip = socket_addr.ip();
            match self.config.address_family {
                AddressFamily::Ipv4 if !ip.is_ipv4() => continue,
                AddressFamily::Ipv6 if !ip.is_ipv6() => continue,
                _ => return Ok((ip, ip.is_ipv6())),
            }
        }

        Err(ProbeExecutionError::Dns(format!(
            "no matching address for {}",
            host
        )))
    }

    fn create_socket(
        &self,
        is_ipv6: bool,
    ) -> Result<(UdpSocket, SocketMethod), ProbeExecutionError> {
        let domain = if is_ipv6 { Domain::IPV6 } else { Domain::IPV4 };
        let protocol = if is_ipv6 {
            Protocol::ICMPV6
        } else {
            Protocol::ICMPV4
        };

        let (socket, method) = try_create_socket_for_probe(domain, protocol)?;

        let identifier = std::process::id() as u16;
        bind_icmp_socket(&socket, is_ipv6, identifier)?;

        socket
            .set_nonblocking(true)
            .map_err(|e| ProbeExecutionError::Internal(e.to_string()))?;

        let std_socket: std::net::UdpSocket = socket.into();
        let tokio_socket = UdpSocket::from_std(std_socket)
            .map_err(|e| ProbeExecutionError::Internal(e.to_string()))?;

        Ok((tokio_socket, method))
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_and_receive(
        &self,
        socket: &UdpSocket,
        target: IpAddr,
        is_ipv6: bool,
        method: SocketMethod,
        identifier: u16,
        sequence: u16,
        timeout: Duration,
    ) -> SampleResult {
        let request = build_echo_request(is_ipv6, identifier, sequence, self.config.payload_size);
        let target_addr = SocketAddr::new(target, 0);

        let start = std::time::Instant::now();

        if let Err(e) = socket.send_to(&request, target_addr).await {
            return classify_send_error(&e);
        }

        let deadline = start + timeout;
        let mut buf = [0u8; RECV_BUFFER_SIZE];
        let mut attempts = 0;

        loop {
            let now = std::time::Instant::now();
            let remaining = deadline.saturating_duration_since(now);
            if remaining.is_zero() {
                return SampleResult {
                    outcome: SampleOutcome::Timeout,
                    latency: Some(start.elapsed()),
                    detail: Some("icmp receive timeout".to_owned()),
                    metadata: None,
                };
            }

            let result = tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await;
            match result {
                Ok(Ok((n, _from))) => {
                    let data = &buf[..n];
                    let elapsed = start.elapsed();
                    match parse_icmp_reply(data, is_ipv6, method, identifier, sequence) {
                        ParsedReply::Match(detail) => {
                            return SampleResult {
                                outcome: SampleOutcome::Success,
                                latency: Some(elapsed),
                                detail: Some(detail),
                                metadata: None,
                            };
                        }
                        ParsedReply::Error(outcome, detail) => {
                            return SampleResult {
                                outcome,
                                latency: Some(elapsed),
                                detail: Some(detail),
                                metadata: None,
                            };
                        }
                        ParsedReply::Retry => {
                            attempts += 1;
                            if attempts >= MAX_RECV_ATTEMPTS {
                                return SampleResult {
                                    outcome: SampleOutcome::Timeout,
                                    latency: Some(start.elapsed()),
                                    detail: Some("icmp max retry attempts".to_owned()),
                                    metadata: None,
                                };
                            }
                            continue;
                        }
                    }
                }
                Ok(Err(e)) => return classify_recv_error(&e, start.elapsed()),
                Err(_) => {
                    return SampleResult {
                        outcome: SampleOutcome::Timeout,
                        latency: Some(start.elapsed()),
                        detail: Some("icmp receive timeout".to_owned()),
                        metadata: None,
                    };
                }
            }
        }
    }
}

fn try_create_socket_for_probe(
    domain: Domain,
    protocol: Protocol,
) -> Result<(Socket, SocketMethod), ProbeExecutionError> {
    if let Ok(socket) = Socket::new(domain, Type::DGRAM, Some(protocol)) {
        return Ok((socket, SocketMethod::Dgram));
    }

    Socket::new(domain, Type::RAW, Some(protocol))
        .map(|s| (s, SocketMethod::Raw))
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                ProbeExecutionError::Permission("ICMP socket creation denied".to_owned())
            } else {
                ProbeExecutionError::Internal(format!("ICMP socket error: {}", e))
            }
        })
}

fn bind_icmp_socket(
    socket: &Socket,
    is_ipv6: bool,
    identifier: u16,
) -> Result<(), ProbeExecutionError> {
    let addr = if is_ipv6 {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), identifier)
    } else {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), identifier)
    };
    let sock_addr = socket2::SockAddr::from(addr);
    socket
        .bind(&sock_addr)
        .map_err(|e| ProbeExecutionError::Internal(format!("ICMP bind error: {}", e)))
}

enum ParsedReply {
    Match(String),
    Error(SampleOutcome, String),
    Retry,
}

fn parse_icmp_reply(
    data: &[u8],
    is_ipv6: bool,
    method: SocketMethod,
    expected_identifier: u16,
    expected_sequence: u16,
) -> ParsedReply {
    let icmp_data = match method {
        SocketMethod::Dgram => data,
        SocketMethod::Raw if !is_ipv6 => {
            if data.len() < 20 {
                return ParsedReply::Retry;
            }
            let ihl = ((data[0] & 0x0f) as usize) * 4;
            if data.len() < ihl + 8 {
                return ParsedReply::Retry;
            }
            &data[ihl..]
        }
        SocketMethod::Raw => data,
    };

    if icmp_data.len() < 8 {
        return ParsedReply::Retry;
    }

    let icmp_type = icmp_data[0];
    let icmp_code = icmp_data[1];
    let identifier = u16::from_be_bytes([icmp_data[4], icmp_data[5]]);
    let sequence = u16::from_be_bytes([icmp_data[6], icmp_data[7]]);

    let expected_reply_type = if is_ipv6 {
        ICMP_ECHO_REPLY_V6
    } else {
        ICMP_ECHO_REPLY_V4
    };

    if icmp_type == expected_reply_type
        && icmp_code == 0
        && identifier == expected_identifier
        && sequence == expected_sequence
    {
        return ParsedReply::Match(format!(
            "id={} seq={} socket_type={}",
            identifier, sequence, method
        ));
    }

    if is_icmp_error_type(icmp_type, is_ipv6) {
        let outcome = classify_icmp_error(icmp_type, is_ipv6);
        return ParsedReply::Error(
            outcome,
            format!("icmp_type={} icmp_code={}", icmp_type, icmp_code),
        );
    }

    ParsedReply::Retry
}

fn is_icmp_error_type(icmp_type: u8, is_ipv6: bool) -> bool {
    if is_ipv6 {
        matches!(icmp_type, ICMP_DEST_UNREACH_V6 | ICMP_TIME_EXCEEDED_V6)
    } else {
        matches!(icmp_type, ICMP_DEST_UNREACH_V4 | ICMP_TIME_EXCEEDED_V4)
    }
}

fn classify_icmp_error(icmp_type: u8, is_ipv6: bool) -> SampleOutcome {
    if is_ipv6 {
        match icmp_type {
            ICMP_DEST_UNREACH_V6 | ICMP_TIME_EXCEEDED_V6 => SampleOutcome::NetworkUnreachable,
            _ => SampleOutcome::UnexpectedResponse,
        }
    } else {
        match icmp_type {
            ICMP_DEST_UNREACH_V4 | ICMP_TIME_EXCEEDED_V4 => SampleOutcome::NetworkUnreachable,
            _ => SampleOutcome::UnexpectedResponse,
        }
    }
}

fn build_echo_request(
    is_ipv6: bool,
    identifier: u16,
    sequence: u16,
    payload_size: usize,
) -> Vec<u8> {
    let icmp_type = if is_ipv6 {
        ICMP_ECHO_REQUEST_V6
    } else {
        ICMP_ECHO_REQUEST_V4
    };
    let mut packet = Vec::with_capacity(8 + payload_size);
    packet.push(icmp_type);
    packet.push(0);
    packet.extend_from_slice(&[0, 0]);
    packet.extend_from_slice(&identifier.to_be_bytes());
    packet.extend_from_slice(&sequence.to_be_bytes());
    for i in 0..payload_size {
        packet.push((i % 256) as u8);
    }
    let checksum = compute_checksum(&packet);
    packet[2..4].copy_from_slice(&checksum.to_be_bytes());
    packet
}

fn compute_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let len = data.len();
    let mut i = 0;
    while i + 1 < len {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < len {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !sum as u16
}

fn classify_send_error(e: &std::io::Error) -> SampleResult {
    let outcome = if e.kind() == std::io::ErrorKind::PermissionDenied {
        SampleOutcome::PermissionDenied
    } else if e.kind() == std::io::ErrorKind::NetworkUnreachable {
        SampleOutcome::NetworkUnreachable
    } else {
        let msg = e.to_string().to_lowercase();
        if msg.contains("unreachable") {
            SampleOutcome::NetworkUnreachable
        } else if msg.contains("permission") {
            SampleOutcome::PermissionDenied
        } else {
            SampleOutcome::InternalError
        }
    };
    SampleResult {
        outcome,
        latency: None,
        detail: Some(format!("icmp send error: {}", e)),
        metadata: None,
    }
}

fn classify_recv_error(e: &std::io::Error, elapsed: Duration) -> SampleResult {
    let outcome = if e.kind() == std::io::ErrorKind::PermissionDenied {
        SampleOutcome::PermissionDenied
    } else if e.kind() == std::io::ErrorKind::NetworkUnreachable {
        SampleOutcome::NetworkUnreachable
    } else {
        SampleOutcome::InternalError
    };
    SampleResult {
        outcome,
        latency: Some(elapsed),
        detail: Some(format!("icmp recv error: {}", e)),
        metadata: None,
    }
}

#[async_trait]
impl Probe for IcmpProbe {
    fn kind(&self) -> ProbeKind {
        ProbeKind::Icmp
    }

    async fn execute_round(
        &self,
        _context: RoundContext,
        check: ResolvedCheck,
    ) -> Result<ProbeRound, ProbeExecutionError> {
        let (target_ip, is_ipv6) = self.resolve_host(&check.address).await?;
        let (socket, method) = self.create_socket(is_ipv6)?;
        let identifier = std::process::id() as u16;

        let sample_count = check.sample_count.max(1);
        let timeout_per_sample = check.timeout / sample_count;
        let sample_spacing = Duration::from_millis(100);

        let mut results = Vec::with_capacity(sample_count as usize);

        for seq in 0..sample_count {
            if seq > 0 {
                tokio::time::sleep(sample_spacing).await;
            }
            let result = self
                .send_and_receive(
                    &socket,
                    target_ip,
                    is_ipv6,
                    method,
                    identifier,
                    seq as u16,
                    timeout_per_sample,
                )
                .await;
            results.push(result);
        }

        Ok(ProbeRound {
            check_id: check.check_id,
            results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_checksum_empty() {
        let checksum = compute_checksum(&[]);
        assert_eq!(checksum, 0xffff);
    }

    #[test]
    fn compute_checksum_known_value() {
        let mut packet = vec![8u8, 0, 0, 0, 0, 1, 0, 1];
        let checksum = compute_checksum(&packet);
        packet[2..4].copy_from_slice(&checksum.to_be_bytes());
        let verify = compute_checksum(&packet);
        assert_eq!(verify, 0);
    }

    #[test]
    fn build_echo_request_ipv4_checksum_valid() {
        let packet = build_echo_request(false, 0x1234, 1, 4);
        assert_eq!(packet[0], ICMP_ECHO_REQUEST_V4);
        assert_eq!(packet[1], 0);
        assert_eq!(packet.len(), 12);
        assert_eq!(&packet[4..6], &0x1234u16.to_be_bytes());
        assert_eq!(&packet[6..8], &1u16.to_be_bytes());
        let verify = compute_checksum(&packet);
        assert_eq!(verify, 0);
    }

    #[test]
    fn build_echo_request_ipv6_checksum_valid() {
        let packet = build_echo_request(true, 0x5678, 2, 8);
        assert_eq!(packet[0], ICMP_ECHO_REQUEST_V6);
        assert_eq!(packet[1], 0);
        assert_eq!(packet.len(), 16);
        assert_eq!(&packet[4..6], &0x5678u16.to_be_bytes());
        assert_eq!(&packet[6..8], &2u16.to_be_bytes());
        let verify = compute_checksum(&packet);
        assert_eq!(verify, 0);
    }

    #[test]
    fn build_echo_request_large_payload() {
        let packet = build_echo_request(false, 1, 1, DEFAULT_PAYLOAD_SIZE);
        assert_eq!(packet.len(), 8 + DEFAULT_PAYLOAD_SIZE);
        let verify = compute_checksum(&packet);
        assert_eq!(verify, 0);
    }

    #[test]
    fn parse_echo_reply_ipv4_dgram_match() {
        let request = build_echo_request(false, 0xabcd, 5, 8);
        let mut reply = request.clone();
        reply[0] = ICMP_ECHO_REPLY_V4;
        reply[2] = 0;
        reply[3] = 0;
        let checksum = compute_checksum(&reply);
        reply[2..4].copy_from_slice(&checksum.to_be_bytes());

        let result = parse_icmp_reply(&reply, false, SocketMethod::Dgram, 0xabcd, 5);
        assert!(matches!(result, ParsedReply::Match(_)));
    }

    #[test]
    fn parse_echo_reply_ipv6_dgram_match() {
        let request = build_echo_request(true, 0xef01, 3, 16);
        let mut reply = request.clone();
        reply[0] = ICMP_ECHO_REPLY_V6;
        reply[2] = 0;
        reply[3] = 0;
        let checksum = compute_checksum(&reply);
        reply[2..4].copy_from_slice(&checksum.to_be_bytes());

        let result = parse_icmp_reply(&reply, true, SocketMethod::Dgram, 0xef01, 3);
        assert!(matches!(result, ParsedReply::Match(_)));
    }

    #[test]
    fn parse_echo_reply_identifier_mismatch() {
        let request = build_echo_request(false, 0x1111, 1, 8);
        let mut reply = request.clone();
        reply[0] = ICMP_ECHO_REPLY_V4;
        reply[4] = 0x22;
        reply[5] = 0x22;
        reply[2] = 0;
        reply[3] = 0;
        let checksum = compute_checksum(&reply);
        reply[2..4].copy_from_slice(&checksum.to_be_bytes());

        let result = parse_icmp_reply(&reply, false, SocketMethod::Dgram, 0x1111, 1);
        assert!(matches!(result, ParsedReply::Retry));
    }

    #[test]
    fn parse_echo_reply_sequence_mismatch() {
        let request = build_echo_request(false, 0x1111, 1, 8);
        let mut reply = request.clone();
        reply[0] = ICMP_ECHO_REPLY_V4;
        reply[6] = 0;
        reply[7] = 99;
        reply[2] = 0;
        reply[3] = 0;
        let checksum = compute_checksum(&reply);
        reply[2..4].copy_from_slice(&checksum.to_be_bytes());

        let result = parse_icmp_reply(&reply, false, SocketMethod::Dgram, 0x1111, 1);
        assert!(matches!(result, ParsedReply::Retry));
    }

    #[test]
    fn parse_dest_unreachable_ipv4() {
        let mut reply = vec![ICMP_DEST_UNREACH_V4, 0, 0, 0, 0, 0, 0, 0];
        let checksum = compute_checksum(&reply);
        reply[2..4].copy_from_slice(&checksum.to_be_bytes());

        let result = parse_icmp_reply(&reply, false, SocketMethod::Dgram, 0x1111, 1);
        match result {
            ParsedReply::Error(outcome, _) => {
                assert_eq!(outcome, SampleOutcome::NetworkUnreachable);
            }
            _ => panic!("expected error reply"),
        }
    }

    #[test]
    fn parse_time_exceeded_ipv6() {
        let mut reply = vec![ICMP_TIME_EXCEEDED_V6, 0, 0, 0, 0, 0, 0, 0];
        let checksum = compute_checksum(&reply);
        reply[2..4].copy_from_slice(&checksum.to_be_bytes());

        let result = parse_icmp_reply(&reply, true, SocketMethod::Dgram, 0x1111, 1);
        match result {
            ParsedReply::Error(outcome, _) => {
                assert_eq!(outcome, SampleOutcome::NetworkUnreachable);
            }
            _ => panic!("expected error reply"),
        }
    }

    #[test]
    fn parse_ipv4_raw_with_ip_header() {
        let request = build_echo_request(false, 0xabcd, 1, 8);
        let mut reply = request.clone();
        reply[0] = ICMP_ECHO_REPLY_V4;
        reply[2] = 0;
        reply[3] = 0;
        let checksum = compute_checksum(&reply);
        reply[2..4].copy_from_slice(&checksum.to_be_bytes());

        let mut ip_packet = vec![0u8; 20];
        ip_packet[0] = 0x45;
        ip_packet.extend_from_slice(&reply);

        let result = parse_icmp_reply(&ip_packet, false, SocketMethod::Raw, 0xabcd, 1);
        assert!(matches!(result, ParsedReply::Match(_)));
    }

    #[test]
    fn parse_too_short_packet() {
        let data = [0u8; 4];
        let result = parse_icmp_reply(&data, false, SocketMethod::Dgram, 1, 1);
        assert!(matches!(result, ParsedReply::Retry));
    }

    #[test]
    fn parse_ipv4_raw_too_short_ip_header() {
        let data = [0x45u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = parse_icmp_reply(&data, false, SocketMethod::Raw, 1, 1);
        assert!(matches!(result, ParsedReply::Retry));
    }

    #[test]
    fn default_config() {
        let config = IcmpProbeConfig::default();
        assert_eq!(config.address_family, AddressFamily::Auto);
        assert_eq!(config.payload_size, DEFAULT_PAYLOAD_SIZE);
        assert!(config.source_address.is_none());
    }

    #[test]
    fn socket_method_display() {
        assert_eq!(SocketMethod::Dgram.to_string(), "dgram");
        assert_eq!(SocketMethod::Raw.to_string(), "raw");
    }

    #[test]
    fn icmp_capability_check() {
        let cap = check_icmp_capability();
        let _ = cap.is_available();
    }
}
