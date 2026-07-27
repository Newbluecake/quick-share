use quick_share_discovery::{
    Advertisement, Discovery, DiscoveryError, FakeDiscovery, InterfaceAddress, MdnsDiscovery,
    MdnsRegistration, RawService, ScanRequest, ScanResult, TxtRecord, aggregate_services,
    filter_lan_interfaces,
};
use quick_share_protocol::{Capability, DeviceId, ProtocolVersion};
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

fn id(value: char) -> DeviceId {
    DeviceId::parse(format!("qs_{}", value.to_string().repeat(32))).expect("device ID")
}

fn advertisement() -> Advertisement {
    Advertisement {
        device_id: id('1'),
        name: "laptop\nname".to_owned(),
        version: ProtocolVersion::V1_0,
        capabilities: BTreeSet::from([
            Capability::Files,
            Capability::Directories,
            Capability::Resume,
        ]),
        static_key_fingerprint: [7; 32],
        port: 4242,
    }
}

#[test]
fn txt_record_round_trip_is_bounded_and_removes_control_characters() {
    let encoded = TxtRecord::from_advertisement(&advertisement())
        .expect("TXT record")
        .encode();
    let decoded = TxtRecord::decode(&encoded).expect("decode TXT record");

    assert_eq!(decoded.device_id, id('1'));
    assert_eq!(decoded.name, "laptopname");
    assert_eq!(decoded.port, 4242);
    assert!(decoded.capabilities.contains(&Capability::Resume));
    assert!(encoded.values().all(|value| value.len() <= 255));
}

#[test]
fn malformed_unknown_or_oversized_txt_fields_are_rejected() {
    let mut encoded = TxtRecord::from_advertisement(&advertisement())
        .expect("TXT record")
        .encode();
    encoded.insert("admin".to_owned(), "true".to_owned());
    assert!(TxtRecord::decode(&encoded).is_err());

    let mut oversized = TxtRecord::from_advertisement(&advertisement())
        .expect("TXT record")
        .encode();
    oversized.insert("name".to_owned(), "x".repeat(256));
    assert!(TxtRecord::decode(&oversized).is_err());
}

#[test]
fn aggregation_excludes_self_incompatible_and_duplicate_services() {
    let properties = TxtRecord::from_advertisement(&advertisement())
        .expect("TXT")
        .encode();
    let peer_properties = {
        let mut record = advertisement();
        record.device_id = id('2');
        TxtRecord::from_advertisement(&record)
            .expect("peer TXT")
            .encode()
    };
    let incompatible = {
        let mut record = advertisement();
        record.device_id = id('3');
        record.version = ProtocolVersion::new(2, 0);
        TxtRecord::from_advertisement(&record)
            .expect("incompatible TXT")
            .encode()
    };
    let raws = vec![
        RawService::new(properties, ["192.168.1.2".parse().expect("IP")], 4242),
        RawService::new(
            peer_properties.clone(),
            ["192.168.1.3".parse().expect("IP")],
            4242,
        ),
        RawService::new(peer_properties, ["192.168.1.4".parse().expect("IP")], 4242),
        RawService::new(incompatible, ["192.168.1.5".parse().expect("IP")], 4242),
    ];

    let result = aggregate_services(&id('1'), raws, Vec::new(), true);

    assert_eq!(result.peers.len(), 1);
    assert_eq!(result.peers[0].device_id, id('2'));
    assert_eq!(result.peers[0].endpoints.len(), 2);
    assert!(result.peers[0].unverified);
}

#[test]
fn interface_filter_removes_loopback_down_p2p_and_virtual_by_default() {
    let candidates = vec![
        InterfaceAddress::new("eth0", "192.168.1.10".parse().expect("IP"), true, false),
        InterfaceAddress::new("docker0", "172.17.0.1".parse().expect("IP"), true, false),
        InterfaceAddress::new("lo", IpAddr::V4(Ipv4Addr::LOCALHOST), true, false),
        InterfaceAddress::new("down0", "10.0.0.2".parse().expect("IP"), false, false),
        InterfaceAddress::new("tun0", "10.8.0.2".parse().expect("IP"), true, true),
    ];

    let normal = filter_lan_interfaces(candidates.clone(), false);
    let including_virtual = filter_lan_interfaces(candidates, true);

    assert_eq!(normal.len(), 1);
    assert_eq!(normal[0].name, "eth0");
    assert!(including_virtual.iter().any(|item| item.name == "docker0"));
}

#[tokio::test]
#[ignore = "requires a true-host multicast-capable LAN interface"]
async fn true_host_mdns_registration_and_scan_interoperate() {
    let advertised = advertisement();
    let _registration = MdnsRegistration::start(&advertised, false).expect("register mDNS");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let result = MdnsDiscovery::new(false)
        .scan(ScanRequest {
            local_device_id: id('9'),
            timeout: Duration::from_secs(3),
        })
        .await
        .expect("scan mDNS");

    assert!(
        result
            .peers
            .iter()
            .any(|peer| peer.device_id == advertised.device_id)
    );
}

#[tokio::test]
async fn fake_discovery_distinguishes_definitive_empty_partial_and_failure() {
    let empty = ScanResult::complete(Vec::new());
    let partial = ScanResult::partial(
        vec![quick_share_discovery::UnverifiedPeer {
            device_id: id('2'),
            name: "peer".to_owned(),
            version: ProtocolVersion::V1_0,
            capabilities: BTreeSet::new(),
            static_key_fingerprint: [2; 32],
            endpoints: BTreeSet::from([SocketAddr::from(([192, 0, 2, 10], 4242))]),
            unverified: true,
        }],
        vec!["one interface failed".to_owned()],
    );
    let backend = FakeDiscovery::new([
        Ok(empty),
        Ok(partial),
        Err(DiscoveryError::Backend(
            "multicast permission denied; use --peer host:port".to_owned(),
        )),
    ]);
    let request = ScanRequest {
        local_device_id: id('1'),
        timeout: Duration::from_millis(10),
    };

    assert!(
        backend
            .scan(request.clone())
            .await
            .expect("empty")
            .is_definitive_empty()
    );
    let partial = backend.scan(request.clone()).await.expect("partial");
    assert!(!partial.complete);
    assert_eq!(partial.peers.len(), 1);
    let error = backend.scan(request).await.expect_err("discovery failure");
    assert!(error.to_string().contains("--peer"));
}
