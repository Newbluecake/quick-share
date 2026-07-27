use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    thread,
    time::{Duration, Instant},
};

const SERVICE_TYPE: &str = "_quickshare._tcp.local.";

#[derive(Debug, Parser)]
#[command(about = "Disposable Quick Share mDNS two-machine experiment")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Advertise {
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = 53317)]
        port: u16,
        #[arg(long, default_value_t = 120)]
        duration: u64,
        /// Publish only this address. Omitting it demonstrates addr_auto behavior.
        #[arg(long)]
        ip: Option<IpAddr>,
    },
    Scan {
        #[arg(long, default_value_t = 10)]
        timeout: u64,
        #[arg(long)]
        expect_id: Option<String>,
    },
}

#[derive(Debug, Serialize)]
struct FoundPeer {
    fullname: String,
    hostname: String,
    addresses: Vec<String>,
    port: u16,
    properties: BTreeMap<String, String>,
    elapsed_ms: u128,
}

fn sanitized_instance(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control())
        .take(40)
        .collect::<String>()
        .replace('.', "-")
}

fn advertise(name: String, port: u16, duration: u64, ip: Option<IpAddr>) -> Result<()> {
    let instance = sanitized_instance(&name);
    ensure!(!instance.is_empty(), "name is empty after sanitization");
    let device_id = format!("spike-{}", instance.to_lowercase().replace(' ', "-"));
    let host_name = format!("{device_id}.local.");
    let properties = [
        ("id", device_id.as_str()),
        ("name", instance.as_str()),
        ("ver", "1"),
        ("caps", "spike"),
    ];

    let daemon = ServiceDaemon::new().context("create mDNS daemon")?;
    let service = match ip {
        Some(ip) => ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host_name,
            ip,
            port,
            &properties[..],
        )
        .context("create explicit-address mDNS service")?,
        None => ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &host_name,
            (),
            port,
            &properties[..],
        )
        .context("create automatic-address mDNS service")?
        .enable_addr_auto(),
    };
    let fullname = service.get_fullname().to_owned();
    daemon.register(service).context("register mDNS service")?;

    println!("ADVERTISING");
    println!("  service: {fullname}");
    println!("  id:      {device_id}");
    println!("  port:    {port}");
    println!(
        "  address: {}",
        ip.map_or_else(
            || "automatic (experimental)".to_owned(),
            |value| value.to_string()
        )
    );
    println!("  seconds: {duration}");
    println!("Keep this process running while the other machine scans.");
    thread::sleep(Duration::from_secs(duration));

    let _ = daemon.unregister(&fullname);
    let _ = daemon.shutdown();
    Ok(())
}

fn scan(timeout: u64, expect_id: Option<String>) -> Result<()> {
    let daemon = ServiceDaemon::new().context("create mDNS daemon")?;
    let receiver = daemon.browse(SERVICE_TYPE).context("start mDNS browse")?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(timeout);
    let mut peers = BTreeMap::<String, FoundPeer>::new();

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining.min(Duration::from_millis(500))) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                let properties = info
                    .get_properties()
                    .iter()
                    .map(|property| (property.key().to_owned(), property.val_str().to_owned()))
                    .collect();
                let peer = FoundPeer {
                    fullname: info.get_fullname().to_owned(),
                    hostname: info.get_hostname().to_owned(),
                    addresses: info
                        .get_addresses()
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    port: info.get_port(),
                    properties,
                    elapsed_ms: started.elapsed().as_millis(),
                };
                peers.insert(peer.fullname.clone(), peer);
            }
            Ok(ServiceEvent::SearchStopped(_)) => break,
            Ok(_) => {}
            Err(_) => {}
        }
    }

    daemon
        .stop_browse(SERVICE_TYPE)
        .context("stop mDNS browse")?;
    let _ = daemon.shutdown();
    let output: Vec<_> = peers.into_values().collect();
    println!("{}", serde_json::to_string_pretty(&output)?);

    if let Some(expected) = expect_id {
        ensure!(
            output
                .iter()
                .any(|peer| peer.properties.get("id") == Some(&expected)),
            "expected device id {expected:?} was not discovered"
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Advertise {
            name,
            port,
            duration,
            ip,
        } => advertise(name, port, duration, ip),
        Command::Scan { timeout, expect_id } => scan(timeout, expect_id),
    }
}

#[cfg(test)]
mod tests {
    use super::sanitized_instance;

    #[test]
    fn instance_name_removes_control_characters_and_dots() {
        // Arrange + Act
        let name = sanitized_instance("hello.\nworld");

        // Assert
        assert_eq!(name, "hello-world");
    }

    #[test]
    fn instance_name_has_a_hard_length_limit() {
        // Arrange + Act
        let name = sanitized_instance(&"a".repeat(100));

        // Assert
        assert_eq!(name.len(), 40);
    }
}
