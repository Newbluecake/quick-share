//! Trusted-device command implementation without exposing complete static public keys.

use crate::{AppError, DevicesIntent, InteractionPolicy};
use quick_share_core::identity::{TrustedDevice, TrustedDeviceStore};
use quick_share_protocol::DeviceId;
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceCommandOutcome {
    Listed(usize),
    Renamed(DeviceId),
    Removed(DeviceId),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceView<'a> {
    device_id: &'a str,
    name: &'a str,
    fingerprint: String,
}

pub fn run_devices(
    store: &TrustedDeviceStore,
    intent: &DevicesIntent,
    interaction: InteractionPolicy,
    output: &mut dyn Write,
) -> Result<DeviceCommandOutcome, AppError> {
    match intent {
        DevicesIntent::List { json } => {
            let mut devices = store.list().map_err(map_store)?;
            devices.sort_by(|left, right| {
                left.name
                    .to_lowercase()
                    .cmp(&right.name.to_lowercase())
                    .then_with(|| left.device_id.cmp(&right.device_id))
            });
            if *json {
                let views = devices.iter().map(view).collect::<Vec<_>>();
                serde_json::to_writer_pretty(&mut *output, &views)
                    .map_err(|error| AppError::Filesystem(error.to_string()))?;
                writeln!(output).map_err(map_output)?;
            } else if devices.is_empty() {
                writeln!(output, "No trusted devices.").map_err(map_output)?;
            } else {
                writeln!(output, "TRUSTED DEVICE\tDEVICE ID\tFINGERPRINT").map_err(map_output)?;
                for device in &devices {
                    let item = view(device);
                    writeln!(
                        output,
                        "{}\t{}\t{}",
                        item.name, item.device_id, item.fingerprint
                    )
                    .map_err(map_output)?;
                }
            }
            Ok(DeviceCommandOutcome::Listed(devices.len()))
        }
        DevicesIntent::Rename { device, name } => {
            let id = DeviceId::parse(device).map_err(|error| AppError::Usage(error.to_string()))?;
            store.rename(&id, name).map_err(map_store)?;
            writeln!(output, "Renamed trusted device {id}.").map_err(map_output)?;
            Ok(DeviceCommandOutcome::Renamed(id))
        }
        DevicesIntent::Remove {
            device,
            assume_yes: _,
        } => {
            interaction.require_confirmation("remove trusted device")?;
            let id = DeviceId::parse(device).map_err(|error| AppError::Usage(error.to_string()))?;
            if !store.remove(&id).map_err(map_store)? {
                return Err(AppError::Usage(format!(
                    "trusted device {id} was not found"
                )));
            }
            writeln!(output, "Removed trusted device {id}.").map_err(map_output)?;
            Ok(DeviceCommandOutcome::Removed(id))
        }
    }
}

fn view(device: &TrustedDevice) -> DeviceView<'_> {
    DeviceView {
        device_id: device.device_id.as_str(),
        name: &device.name,
        fingerprint: hex_prefix(&device.public_key),
    }
}

fn hex_prefix(public_key: &[u8; 32]) -> String {
    let digest = blake3::derive_key("quick-share/qsp1/trusted-device-fingerprint", public_key);
    hex::encode(&digest[..6])
}

fn map_store(error: impl std::fmt::Display) -> AppError {
    AppError::Identity(error.to_string())
}

fn map_output(error: std::io::Error) -> AppError {
    AppError::Filesystem(error.to_string())
}
