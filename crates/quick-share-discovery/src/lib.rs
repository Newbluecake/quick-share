#![forbid(unsafe_code)]
//! Link-local Quick Share peer discovery with explicit interface selection.

use async_trait::async_trait;
use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
use quick_share_protocol::{Capabilities, Capability, DeviceId, PROTOCOL_MAJOR, ProtocolVersion};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};
use thiserror::Error;

/// DNS-SD service type for QSP peers.
pub const SERVICE_TYPE: &str = "_quickshare._tcp.local.";
const MAX_TXT_VALUE_BYTES: usize = 255;
const MAX_NAME_BYTES: usize = 240;

/// Public data advertised over untrusted mDNS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertisement {
    pub device_id: DeviceId,
    pub name: String,
    pub version: ProtocolVersion,
    pub capabilities: Capabilities,
    pub static_key_fingerprint: [u8; 32],
    pub port: u16,
}

/// Strict TXT representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtRecord {
    pub device_id: DeviceId,
    pub name: String,
    pub version: ProtocolVersion,
    pub capabilities: Capabilities,
    pub static_key_fingerprint: [u8; 32],
    pub port: u16,
}

impl TxtRecord {
    pub fn from_advertisement(value: &Advertisement) -> Result<Self, DiscoveryError> {
        let name = sanitize_name(&value.name);
        if name.is_empty() || value.port == 0 {
            return Err(DiscoveryError::InvalidRecord(
                "device name and port must be non-empty".to_owned(),
            ));
        }
        Ok(Self {
            device_id: value.device_id.clone(),
            name,
            version: value.version,
            capabilities: value.capabilities.clone(),
            static_key_fingerprint: value.static_key_fingerprint,
            port: value.port,
        })
    }

    #[must_use]
    pub fn encode(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("id".to_owned(), self.device_id.to_string()),
            ("name".to_owned(), self.name.clone()),
            (
                "ver".to_owned(),
                format!("{}.{}", self.version.major, self.version.minor),
            ),
            ("port".to_owned(), self.port.to_string()),
            ("fp".to_owned(), hex::encode(self.static_key_fingerprint)),
            ("caps".to_owned(), encode_capabilities(&self.capabilities)),
        ])
    }

    pub fn decode(values: &BTreeMap<String, String>) -> Result<Self, DiscoveryError> {
        const KEYS: [&str; 6] = ["id", "name", "ver", "port", "fp", "caps"];
        if values.len() != KEYS.len() || values.keys().any(|key| !KEYS.contains(&key.as_str())) {
            return Err(DiscoveryError::InvalidRecord(
                "unknown or missing TXT fields".to_owned(),
            ));
        }
        if values
            .values()
            .any(|value| value.len() > MAX_TXT_VALUE_BYTES)
        {
            return Err(DiscoveryError::InvalidRecord(
                "TXT field exceeds 255 bytes".to_owned(),
            ));
        }
        let device_id = DeviceId::parse(required(values, "id")?)
            .map_err(|error| DiscoveryError::InvalidRecord(error.to_string()))?;
        let name = required(values, "name")?.to_owned();
        if sanitize_name(&name) != name || name.is_empty() {
            return Err(DiscoveryError::InvalidRecord(
                "invalid device display name".to_owned(),
            ));
        }
        let version = parse_version(required(values, "ver")?)?;
        let port = required(values, "port")?
            .parse::<u16>()
            .map_err(|error| DiscoveryError::InvalidRecord(error.to_string()))?;
        if port == 0 {
            return Err(DiscoveryError::InvalidRecord(
                "port cannot be zero".to_owned(),
            ));
        }
        let fingerprint: [u8; 32] = hex::decode(required(values, "fp")?)
            .map_err(|error| DiscoveryError::InvalidRecord(error.to_string()))?
            .try_into()
            .map_err(|_| {
                DiscoveryError::InvalidRecord("fingerprint must contain 32 bytes".to_owned())
            })?;
        Ok(Self {
            device_id,
            name,
            version,
            capabilities: decode_capabilities(required(values, "caps")?)?,
            static_key_fingerprint: fingerprint,
            port,
        })
    }
}

fn required<'a>(
    values: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, DiscoveryError> {
    values
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| DiscoveryError::InvalidRecord(format!("missing TXT field {key}")))
}

fn sanitize_name(value: &str) -> String {
    let mut output = String::new();
    for character in value
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
    {
        if output.len() + character.len_utf8() > MAX_NAME_BYTES {
            break;
        }
        output.push(character);
    }
    output
}

fn parse_version(value: &str) -> Result<ProtocolVersion, DiscoveryError> {
    let (major, minor) = value
        .split_once('.')
        .ok_or_else(|| DiscoveryError::InvalidRecord("version must be major.minor".to_owned()))?;
    Ok(ProtocolVersion::new(
        major.parse().map_err(|error: std::num::ParseIntError| {
            DiscoveryError::InvalidRecord(error.to_string())
        })?,
        minor.parse().map_err(|error: std::num::ParseIntError| {
            DiscoveryError::InvalidRecord(error.to_string())
        })?,
    ))
}

fn encode_capabilities(values: &Capabilities) -> String {
    values
        .iter()
        .map(|capability| match capability {
            Capability::Files => "files",
            Capability::Directories => "dirs",
            Capability::Text => "text",
            Capability::Resume => "resume",
            Capability::Symlinks => "symlinks",
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_capabilities(value: &str) -> Result<Capabilities, DiscoveryError> {
    if value.is_empty() {
        return Ok(BTreeSet::new());
    }
    value
        .split(',')
        .map(|item| match item {
            "files" => Ok(Capability::Files),
            "dirs" => Ok(Capability::Directories),
            "text" => Ok(Capability::Text),
            "resume" => Ok(Capability::Resume),
            "symlinks" => Ok(Capability::Symlinks),
            _ => Err(DiscoveryError::InvalidRecord(format!(
                "unknown capability {item:?}"
            ))),
        })
        .collect()
}

/// Testable interface facts independent from an OS-specific adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceAddress {
    pub name: String,
    pub ip: IpAddr,
    pub is_up: bool,
    pub is_p2p: bool,
}

impl InterfaceAddress {
    #[must_use]
    pub fn new(name: impl Into<String>, ip: IpAddr, is_up: bool, is_p2p: bool) -> Self {
        Self {
            name: name.into(),
            ip,
            is_up,
            is_p2p,
        }
    }
}

/// Enumerates operational interface addresses and copies only non-sensitive facts.
pub fn system_interfaces() -> Result<Vec<InterfaceAddress>, DiscoveryError> {
    if_addrs::get_if_addrs()
        .map(|items| {
            items
                .into_iter()
                .map(|item| {
                    let ip = item.ip();
                    let is_up = item.is_oper_up();
                    let is_p2p = item.is_p2p();
                    InterfaceAddress::new(item.name, ip, is_up, is_p2p)
                })
                .collect()
        })
        .map_err(|error| DiscoveryError::Interfaces(error.to_string()))
}

/// Filters loopback, unusable, point-to-point, and common virtual interfaces.
#[must_use]
pub fn filter_lan_interfaces(
    interfaces: Vec<InterfaceAddress>,
    include_virtual: bool,
) -> Vec<InterfaceAddress> {
    let mut seen = BTreeSet::new();
    interfaces
        .into_iter()
        .filter(|item| {
            item.is_up
                && !item.is_p2p
                && usable_ip(item.ip)
                && (include_virtual || !is_virtual_name(&item.name))
                && seen.insert((item.name.clone(), item.ip))
        })
        .collect()
}

fn usable_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(value) => {
            !value.is_loopback()
                && !value.is_unspecified()
                && !value.is_multicast()
                && !value.is_link_local()
                && value != Ipv4Addr::BROADCAST
        }
        IpAddr::V6(value) => {
            !value.is_loopback()
                && !value.is_unspecified()
                && !value.is_multicast()
                && !value.is_unicast_link_local()
        }
    }
}

fn is_virtual_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "docker",
        "veth",
        "virbr",
        "vmnet",
        "vbox",
        "br-",
        "tun",
        "tap",
        "tailscale",
        "zt",
        "wsl",
        "hyper-v",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

/// Raw resolved event before strict TXT validation and deduplication.
#[derive(Debug, Clone)]
pub struct RawService {
    pub properties: BTreeMap<String, String>,
    pub addresses: BTreeSet<IpAddr>,
    pub srv_port: u16,
}

impl RawService {
    #[must_use]
    pub fn new<I>(properties: BTreeMap<String, String>, addresses: I, srv_port: u16) -> Self
    where
        I: IntoIterator<Item = IpAddr>,
    {
        Self {
            properties,
            addresses: addresses.into_iter().collect(),
            srv_port,
        }
    }
}

/// A discovery hint that remains untrusted until the Noise handshake and INFO exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedPeer {
    pub device_id: DeviceId,
    pub name: String,
    pub version: ProtocolVersion,
    pub capabilities: Capabilities,
    pub static_key_fingerprint: [u8; 32],
    pub endpoints: BTreeSet<SocketAddr>,
    pub unverified: bool,
}

/// Scan output differentiating complete zero results from partial failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    pub peers: Vec<UnverifiedPeer>,
    pub warnings: Vec<String>,
    pub complete: bool,
}

impl ScanResult {
    #[must_use]
    pub const fn complete(peers: Vec<UnverifiedPeer>) -> Self {
        Self {
            peers,
            warnings: Vec::new(),
            complete: true,
        }
    }

    #[must_use]
    pub const fn partial(peers: Vec<UnverifiedPeer>, warnings: Vec<String>) -> Self {
        Self {
            peers,
            warnings,
            complete: false,
        }
    }

    /// Only this state permits automatic fallback to Web mode.
    #[must_use]
    pub fn is_definitive_empty(&self) -> bool {
        self.complete && self.peers.is_empty()
    }
}

/// Combines multi-interface events by key-derived device ID and endpoint.
#[must_use]
pub fn aggregate_services(
    local_device_id: &DeviceId,
    services: Vec<RawService>,
    mut warnings: Vec<String>,
    complete: bool,
) -> ScanResult {
    let mut peers = BTreeMap::<DeviceId, UnverifiedPeer>::new();
    for service in services {
        let record = match TxtRecord::decode(&service.properties) {
            Ok(record) => record,
            Err(error) => {
                warnings.push(error.to_string());
                continue;
            }
        };
        if &record.device_id == local_device_id || record.version.major != PROTOCOL_MAJOR {
            continue;
        }
        if record.port != service.srv_port {
            warnings.push(format!(
                "ignored {} because TXT and SRV ports differ",
                record.device_id
            ));
            continue;
        }
        let endpoints: BTreeSet<_> = service
            .addresses
            .into_iter()
            .filter(|ip| usable_ip(*ip))
            .map(|ip| SocketAddr::new(ip, record.port))
            .collect();
        if endpoints.is_empty() {
            continue;
        }
        match peers.get_mut(&record.device_id) {
            Some(existing) if existing.static_key_fingerprint == record.static_key_fingerprint => {
                existing.endpoints.extend(endpoints);
            }
            Some(_) => warnings.push(format!(
                "conflicting fingerprints advertised for {}",
                record.device_id
            )),
            None => {
                peers.insert(
                    record.device_id.clone(),
                    UnverifiedPeer {
                        device_id: record.device_id,
                        name: record.name,
                        version: record.version,
                        capabilities: record.capabilities,
                        static_key_fingerprint: record.static_key_fingerprint,
                        endpoints,
                        unverified: true,
                    },
                );
            }
        }
    }
    ScanResult {
        peers: peers.into_values().collect(),
        warnings,
        complete,
    }
}

/// Deterministic scan request.
#[derive(Debug, Clone)]
pub struct ScanRequest {
    pub local_device_id: DeviceId,
    pub timeout: Duration,
}

#[async_trait]
pub trait Discovery: Send + Sync {
    async fn scan(&self, request: ScanRequest) -> Result<ScanResult, DiscoveryError>;
}

/// Queue-backed deterministic backend for orchestration tests.
pub struct FakeDiscovery {
    outcomes: Mutex<VecDeque<Result<ScanResult, DiscoveryError>>>,
}

impl FakeDiscovery {
    #[must_use]
    pub fn new<I>(outcomes: I) -> Self
    where
        I: IntoIterator<Item = Result<ScanResult, DiscoveryError>>,
    {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
        }
    }
}

#[async_trait]
impl Discovery for FakeDiscovery {
    async fn scan(&self, _request: ScanRequest) -> Result<ScanResult, DiscoveryError> {
        self.outcomes
            .lock()
            .map_err(|_| DiscoveryError::Backend("fake discovery lock poisoned".to_owned()))?
            .pop_front()
            .ok_or_else(|| {
                DiscoveryError::Backend("fake discovery has no queued result".to_owned())
            })?
    }
}

/// Production mDNS browser.
#[derive(Debug, Clone)]
pub struct MdnsDiscovery {
    pub include_virtual: bool,
}

impl MdnsDiscovery {
    #[must_use]
    pub const fn new(include_virtual: bool) -> Self {
        Self { include_virtual }
    }
}

#[async_trait]
impl Discovery for MdnsDiscovery {
    async fn scan(&self, request: ScanRequest) -> Result<ScanResult, DiscoveryError> {
        let include_virtual = self.include_virtual;
        tokio::task::spawn_blocking(move || scan_blocking(request, include_virtual))
            .await
            .map_err(|error| DiscoveryError::Backend(error.to_string()))?
    }
}

fn scan_blocking(
    request: ScanRequest,
    include_virtual: bool,
) -> Result<ScanResult, DiscoveryError> {
    let selected = filter_lan_interfaces(system_interfaces()?, include_virtual);
    if selected.is_empty() {
        return Err(DiscoveryError::NoInterfaces);
    }
    let daemon = configured_daemon(&selected)?;
    let receiver = daemon.browse(SERVICE_TYPE).map_err(|error| {
        DiscoveryError::Backend(format!("mDNS browse failed: {error}; use --peer host:port"))
    })?;
    let deadline = Instant::now() + request.timeout;
    let mut raws = Vec::new();
    let mut warnings = Vec::new();
    let mut complete = true;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let properties = info
                    .get_properties()
                    .iter()
                    .map(|property| (property.key().to_owned(), property.val_str().to_owned()))
                    .collect();
                raws.push(RawService::new(
                    properties,
                    info.get_addresses()
                        .iter()
                        .map(|address| address.to_ip_addr()),
                    info.get_port(),
                ));
            }
            Ok(ServiceEvent::SearchStopped(_)) => {
                complete = false;
                warnings.push("mDNS search stopped before timeout".to_owned());
                break;
            }
            Ok(_) => {}
            Err(flume::RecvTimeoutError::Timeout) => {}
            Err(error) => {
                complete = false;
                warnings.push(format!("mDNS receive failed: {error}"));
                break;
            }
        }
    }
    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();
    let result = aggregate_services(&request.local_device_id, raws, warnings, complete);
    if result.peers.is_empty() && !result.complete {
        return Err(DiscoveryError::Backend(
            "mDNS discovery failed before a definitive result; use --peer host:port".to_owned(),
        ));
    }
    Ok(result)
}

fn configured_daemon(selected: &[InterfaceAddress]) -> Result<ServiceDaemon, DiscoveryError> {
    let daemon =
        ServiceDaemon::new().map_err(|error| DiscoveryError::Backend(error.to_string()))?;
    daemon
        .disable_interface(IfKind::All)
        .map_err(|error| DiscoveryError::Backend(error.to_string()))?;
    daemon
        .enable_interface(
            selected
                .iter()
                .map(|item| IfKind::Name(item.name.clone()))
                .collect::<Vec<_>>(),
        )
        .map_err(|error| DiscoveryError::Backend(error.to_string()))?;
    Ok(daemon)
}

/// RAII registration. It never calls `enable_addr_auto()`.
pub struct MdnsRegistration {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsRegistration {
    pub fn start(
        advertisement: &Advertisement,
        include_virtual: bool,
    ) -> Result<Self, DiscoveryError> {
        Self::start_for_ips(advertisement, include_virtual, &[])
    }

    /// Registers only addresses on which the production listener is actually bound.
    /// An empty allowlist preserves the all-selected-interface behavior.
    pub fn start_for_ips(
        advertisement: &Advertisement,
        include_virtual: bool,
        allowed_ips: &[IpAddr],
    ) -> Result<Self, DiscoveryError> {
        let mut selected = filter_lan_interfaces(system_interfaces()?, include_virtual);
        if !allowed_ips.is_empty() {
            selected.retain(|interface| allowed_ips.contains(&interface.ip));
        }
        if selected.is_empty() {
            return Err(DiscoveryError::NoInterfaces);
        }
        let daemon = configured_daemon(&selected)?;
        let service = build_service_info(advertisement, &selected)?;
        let fullname = service.get_fullname().to_owned();
        daemon
            .register(service)
            .map_err(|error| DiscoveryError::Backend(error.to_string()))?;
        Ok(Self { daemon, fullname })
    }
}

fn build_service_info(
    advertisement: &Advertisement,
    selected: &[InterfaceAddress],
) -> Result<ServiceInfo, DiscoveryError> {
    let record = TxtRecord::from_advertisement(advertisement)?;
    let properties = record.encode();
    let short_id = &advertisement.device_id.as_str()[3..11];
    let instance = instance_name(&record.name, short_id);
    let hostname = format!("qs-{}.local.", &advertisement.device_id.as_str()[3..]);
    let addresses: Vec<_> = selected.iter().map(|item| item.ip).collect();
    ServiceInfo::new(
        SERVICE_TYPE,
        &instance,
        &hostname,
        addresses.as_slice(),
        advertisement.port,
        properties
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>()
            .as_slice(),
    )
    .map_err(|error| DiscoveryError::Backend(error.to_string()))
}

fn instance_name(name: &str, short_id: &str) -> String {
    let mut base = String::new();
    for character in name.chars().filter(|character| *character != '.') {
        if base.len() + character.len_utf8() > 40 {
            break;
        }
        base.push(character);
    }
    format!("{base}-{short_id}")
}

impl Drop for MdnsRegistration {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Discovery setup, wire, or runtime failure.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum DiscoveryError {
    #[error("invalid mDNS record: {0}")]
    InvalidRecord(String),
    #[error("cannot enumerate network interfaces: {0}")]
    Interfaces(String),
    #[error("no eligible LAN interface is available; connect a network or use --peer host:port")]
    NoInterfaces,
    #[error("discovery backend error: {0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_service_info_never_enables_automatic_addresses() {
        let advertisement = Advertisement {
            device_id: DeviceId::parse("qs_0123456789abcdef0123456789abcdef").expect("device ID"),
            name: "设备🚀".repeat(40),
            version: ProtocolVersion::V1_0,
            capabilities: BTreeSet::from([Capability::Files]),
            static_key_fingerprint: [3; 32],
            port: 4242,
        };
        let selected = [InterfaceAddress::new(
            "eth0",
            "192.168.1.10".parse().expect("IP"),
            true,
            false,
        )];

        let service = build_service_info(&advertisement, &selected).expect("service info");

        assert!(!service.is_addr_auto());
        assert!(
            service
                .get_fullname()
                .split('.')
                .next()
                .expect("instance")
                .len()
                <= 63
        );
        assert_eq!(
            service.get_addresses(),
            &std::collections::HashSet::from([selected[0].ip])
        );
    }
}
