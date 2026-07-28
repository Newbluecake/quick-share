//! Hybrid mDNS plus authenticated configured-peer discovery.

use async_trait::async_trait;
use quick_share_core::identity::DeviceIdentity;
use quick_share_discovery::{
    Discovery, DiscoveryError, MdnsDiscovery, ScanRequest, ScanResult, UnverifiedPeer,
};
use quick_share_protocol::{DeviceId, InfoRequest};
use quick_share_transfer::direct::ClientConnector;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use tokio::task::JoinSet;

/// Discovers mDNS services and probes configured `host:port` endpoints over Noise/QSP.
pub(crate) struct HybridDiscovery {
    mdns: MdnsDiscovery,
    configured: Vec<String>,
    identity: Arc<DeviceIdentity>,
    local_info: InfoRequest,
    probe_timeout: Duration,
}

impl HybridDiscovery {
    pub(crate) fn new(
        include_virtual: bool,
        configured: Vec<String>,
        identity: Arc<DeviceIdentity>,
        local_info: InfoRequest,
        probe_timeout: Duration,
    ) -> Self {
        Self {
            mdns: MdnsDiscovery::new(include_virtual),
            configured,
            identity,
            local_info,
            probe_timeout,
        }
    }
}

#[async_trait]
impl Discovery for HybridDiscovery {
    async fn scan(&self, request: ScanRequest) -> Result<ScanResult, DiscoveryError> {
        let mdns = self.mdns.scan(request);
        let configured = probe_configured_peers(
            &self.configured,
            Arc::clone(&self.identity),
            self.local_info.clone(),
            self.probe_timeout,
        );
        let (mdns, configured) = tokio::join!(mdns, configured);
        let mut warnings = configured.warnings;
        let (mdns_peers, mut complete) = match mdns {
            Ok(scan) => {
                warnings.extend(scan.warnings);
                (scan.peers, scan.complete)
            }
            Err(error) if !self.configured.is_empty() => {
                warnings.push(format!("mDNS discovery failed: {error}"));
                (Vec::new(), false)
            }
            Err(error) => return Err(error),
        };
        if configured.failed {
            complete = false;
        }
        Ok(ScanResult {
            peers: merge_peers(mdns_peers, configured.peers),
            warnings,
            complete,
        })
    }
}

struct ConfiguredProbeResult {
    peers: Vec<UnverifiedPeer>,
    warnings: Vec<String>,
    failed: bool,
}

async fn probe_configured_peers(
    configured: &[String],
    identity: Arc<DeviceIdentity>,
    local_info: InfoRequest,
    timeout: Duration,
) -> ConfiguredProbeResult {
    let mut tasks = JoinSet::new();
    for configured_peer in configured {
        let configured_peer = configured_peer.clone();
        let identity = Arc::clone(&identity);
        let local_info = local_info.clone();
        tasks.spawn(async move {
            let endpoints = tokio::net::lookup_host(configured_peer.as_str())
                .await
                .map_err(|error| {
                    format!("configured peer {configured_peer} did not resolve: {error}")
                })?
                .collect::<BTreeSet<_>>();
            if endpoints.is_empty() {
                return Err(format!(
                    "configured peer {configured_peer} resolved to no endpoint"
                ));
            }
            let mut failures = Vec::new();
            for endpoint in endpoints {
                let connector = ClientConnector::new(
                    endpoint,
                    Arc::clone(&identity),
                    None,
                    None,
                    local_info.clone(),
                    timeout,
                );
                match connector.connect().await {
                    Ok(mut connected) => {
                        let peer = UnverifiedPeer {
                            device_id: connected.evidence.remote_device_id.clone(),
                            name: connected.remote_info.device.name.clone(),
                            version: connected.remote_info.protocol_version,
                            capabilities: connected.remote_info.device.capabilities.clone(),
                            static_key_fingerprint: connected.evidence.static_key_fingerprint,
                            endpoints: BTreeSet::from([endpoint]),
                            // A probe authenticates this connection, but selection reconnects and
                            // revalidates identity before authorization or data transfer.
                            unverified: true,
                        };
                        connected.session.close().await;
                        return Ok(peer);
                    }
                    Err(error) => failures.push(format!("{endpoint}: {error}")),
                }
            }
            Err(format!(
                "configured peer {configured_peer} is unavailable ({})",
                failures.join("; ")
            ))
        });
    }

    let mut peers = Vec::new();
    let mut warnings = Vec::new();
    let mut failed = false;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Ok(peer)) if peer.device_id != identity.device_id() => peers.push(peer),
            Ok(Ok(_)) => {
                failed = true;
                warnings.push("configured peer resolves to this device; ignored".to_owned());
            }
            Ok(Err(warning)) => {
                failed = true;
                warnings.push(warning);
            }
            Err(error) => {
                failed = true;
                warnings.push(format!("configured peer probe failed: {error}"));
            }
        }
    }
    ConfiguredProbeResult {
        peers,
        warnings,
        failed,
    }
}

fn merge_peers(mdns: Vec<UnverifiedPeer>, configured: Vec<UnverifiedPeer>) -> Vec<UnverifiedPeer> {
    let mut peers = BTreeMap::<DeviceId, UnverifiedPeer>::new();
    for peer in mdns {
        peers.insert(peer.device_id.clone(), peer);
    }
    for mut peer in configured {
        if let Some(existing) = peers.remove(&peer.device_id)
            && existing.static_key_fingerprint == peer.static_key_fingerprint
        {
            peer.endpoints.extend(existing.endpoints);
        }
        // A Noise-authenticated configured probe supersedes an untrusted mDNS collision.
        peers.insert(peer.device_id.clone(), peer);
    }
    peers.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_share_protocol::{Capability, ProtocolVersion};
    use std::net::SocketAddr;

    fn peer(id: char, fingerprint: u8, port: u16) -> UnverifiedPeer {
        UnverifiedPeer {
            device_id: DeviceId::parse(format!("qs_{}", id.to_string().repeat(32)))
                .expect("device id"),
            name: format!("peer-{id}"),
            version: ProtocolVersion::V1_1,
            capabilities: BTreeSet::from([Capability::Files]),
            static_key_fingerprint: [fingerprint; 32],
            endpoints: BTreeSet::from([SocketAddr::from(([192, 0, 2, 1], port))]),
            unverified: true,
        }
    }

    #[test]
    fn configured_results_merge_by_identity_and_supersede_forged_mdns() {
        let mdns = peer('a', 1, 4242);
        let mut matching = peer('a', 1, 5252);
        matching.name = "authenticated-name".to_owned();
        let merged = merge_peers(vec![mdns], vec![matching]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name, "authenticated-name");
        assert_eq!(merged[0].endpoints.len(), 2);

        let forged = peer('b', 1, 4242);
        let authenticated = peer('b', 2, 5252);
        let merged = merge_peers(vec![forged], vec![authenticated]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].static_key_fingerprint, [2; 32]);
        assert_eq!(merged[0].endpoints.len(), 1);
    }
}
