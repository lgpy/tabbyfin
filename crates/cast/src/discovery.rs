//! Finding devices with mDNS (`_googlecast._tcp.local.`).
//!
//! `mdns-sd` runs its own daemon thread and hands events over a flume channel; a thread of ours
//! turns those into [`DiscoveryEvent`]s on a tokio channel, keeping the device list so a repeated
//! resolution is an `Updated` rather than a second `Found`. Dropping the [`Discovery`] shuts the
//! browse and the daemon down.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use tokio::sync::mpsc;

use crate::{Error, Result};

/// The service type every device publishes.
pub const SERVICE_TYPE: &str = "_googlecast._tcp.local.";

/// The port a single device listens on; groups use their own, from the SRV record.
pub const DEFAULT_PORT: u16 = 8009;

/// A device found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The device id, `id` in the TXT record.
    pub id: String,
    /// The friendly name, `fn`. This becomes the receiver session's name in the dashboard.
    pub name: String,
    /// The model, `md`: `Chromecast`, `Google TV`, `Google Cast Group`…
    pub model: String,
    pub address: IpAddr,
    pub port: u16,
    /// The app running on the device, `rs`, when it names one.
    pub running_app: Option<String>,
}

impl Device {
    /// A cast group (several TVs playing in sync) is a device of its own, but the UI shows it
    /// differently and it listens on its own port.
    pub fn is_group(&self) -> bool {
        self.model == "Google Cast Group"
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.address, self.port)
    }

    /// Whether the app the device advertises (`rs`) is Jellyfin's receiver: the cast panel then
    /// offers to reconnect to what it plays. The name is all the ad carries, and it's there
    /// from discovery, where asking each device would cost it a TLS connection
    /// (docs/chromecast.md, *Staying connected*).
    pub fn runs_receiver(&self) -> bool {
        self.running_app
            .as_deref()
            .is_some_and(|app| app.to_ascii_lowercase().contains("jellyfin"))
    }
}

/// What changed in the network's devices. One event per device, not a snapshot: a page keeps its
/// own list, replacing by `id` on `Found` and `Updated` and removing on `Lost`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Found(Device),
    Updated(Device),
    Lost(String),
}

/// A running browse. Drop it to stop discovering.
pub struct Discovery {
    daemon: ServiceDaemon,
}

impl Discovery {
    /// Starts browsing and returns the events. Devices already on the network arrive as `Found`
    /// within a few seconds.
    pub fn start() -> Result<(Self, mpsc::Receiver<DiscoveryEvent>)> {
        let daemon = ServiceDaemon::new().map_err(discovery_error)?;
        let events = daemon.browse(SERVICE_TYPE).map_err(discovery_error)?;
        let (tx, rx) = mpsc::channel(64);

        std::thread::Builder::new()
            .name("cast-discovery".into())
            .spawn(move || {
                let mut devices: HashMap<String, Device> = HashMap::new();
                let mut ids: HashMap<String, String> = HashMap::new();
                while let Ok(event) = events.recv() {
                    let event = match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let txt: HashMap<&str, &str> = info
                                .get_properties()
                                .iter()
                                .map(|property| (property.key(), property.val_str()))
                                .collect();
                            let addresses: Vec<IpAddr> =
                                info.get_addresses().iter().map(|ip| ip.to_ip_addr()).collect();
                            let Some(device) = device_from(&info.fullname, &txt, &addresses, info.port) else {
                                continue;
                            };
                            ids.insert(info.fullname.clone(), device.id.clone());
                            let event = match devices.get(&device.id) {
                                // The device answers its own announcements; report only changes.
                                Some(previous) if *previous == device => continue,
                                Some(_) => DiscoveryEvent::Updated(device.clone()),
                                None => DiscoveryEvent::Found(device.clone()),
                            };
                            devices.insert(device.id.clone(), device);
                            event
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            let Some(id) = ids.remove(&fullname) else { continue };
                            devices.remove(&id);
                            DiscoveryEvent::Lost(id)
                        }
                        _ => continue,
                    };
                    // A closed receiver means nobody is interested any more.
                    if tx.blocking_send(event).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| Error::Discovery(error.to_string()))?;

        Ok((Self { daemon }, rx))
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        // The daemon exits by itself; its channel then ends the thread above.
        let _ = self.daemon.shutdown();
    }
}

fn discovery_error(error: mdns_sd::Error) -> Error {
    Error::Discovery(error.to_string())
}

/// Builds a device from the parts of a resolved service. Pure, so the TXT mapping is tested without
/// a network: `txt` is the TXT record as key/value strings and `addresses` what the host resolved
/// to. IPv4 wins over IPv6, since that's the route that always works.
fn device_from(fullname: &str, txt: &HashMap<&str, &str>, addresses: &[IpAddr], port: u16) -> Option<Device> {
    if port == 0 {
        return None;
    }
    let address = addresses
        .iter()
        .find(|address| address.is_ipv4())
        .or_else(|| addresses.first())?;
    let instance = fullname.split('.').next().unwrap_or(fullname);
    let present = |key: &str| txt.get(key).filter(|value| !value.is_empty()).copied();
    Some(Device {
        id: present("id").unwrap_or(fullname).to_string(),
        name: present("fn").unwrap_or(instance).to_string(),
        model: present("md").unwrap_or_default().to_string(),
        address: *address,
        port,
        running_app: present("rs").map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(pairs: &[(&'static str, &'static str)]) -> HashMap<&'static str, &'static str> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn device_from_txt() {
        let properties = txt(&[
            ("id", "abc123"),
            ("fn", "Living Room TV"),
            ("md", "Chromecast Ultra"),
            ("rs", "Jellyfin"),
            ("ve", "05"),
        ]);
        let ipv4: IpAddr = "192.168.1.20".parse().unwrap();
        let ipv6: IpAddr = "fe80::1".parse().unwrap();
        let device = device_from(
            "Living Room TV._googlecast._tcp.local.",
            &properties,
            &[ipv6, ipv4],
            8009,
        )
        .unwrap();
        assert_eq!(device.id, "abc123");
        assert_eq!(device.name, "Living Room TV");
        assert_eq!(device.model, "Chromecast Ultra");
        assert_eq!(device.address, ipv4);
        assert_eq!(device.port, 8009);
        assert_eq!(device.running_app.as_deref(), Some("Jellyfin"));
        assert_eq!(device.socket_addr(), SocketAddr::new(ipv4, 8009));
        assert!(!device.is_group());
    }

    #[test]
    fn device_falls_back_to_the_service_name() {
        // No id, no friendly name, no model, and only an IPv6 address.
        let ipv6: IpAddr = "fe80::1".parse().unwrap();
        let device = device_from("Some TV._googlecast._tcp.local.", &txt(&[("rs", "")]), &[ipv6], 8009).unwrap();
        assert_eq!(device.id, "Some TV._googlecast._tcp.local.");
        assert_eq!(device.name, "Some TV");
        assert_eq!(device.model, "");
        assert_eq!(device.running_app, None);
        assert_eq!(device.address, ipv6);

        // A service without an address, or on port 0, is not usable.
        assert!(device_from("x", &txt(&[]), &[], 8009).is_none());
        assert!(device_from("x", &txt(&[]), &[ipv6], 0).is_none());
    }

    #[test]
    fn groups_are_told_apart() {
        let ipv4: IpAddr = "192.168.1.30".parse().unwrap();
        let properties = txt(&[("id", "group1"), ("fn", "Whole house"), ("md", "Google Cast Group")]);
        let device = device_from("Whole house._googlecast._tcp.local.", &properties, &[ipv4], 41013).unwrap();
        assert!(device.is_group());
        assert_eq!(device.port, 41013);
    }

    #[test]
    fn the_running_app_says_when_the_receiver_is_up() {
        let ipv4: IpAddr = "192.168.1.20".parse().unwrap();
        let device = |rs: Option<&'static str>| {
            let pairs: Vec<(&'static str, &'static str)> = match rs {
                Some(rs) => vec![("id", "abc123"), ("fn", "Living Room TV"), ("rs", rs)],
                None => vec![("id", "abc123"), ("fn", "Living Room TV")],
            };
            device_from(
                "Living Room TV._googlecast._tcp.local.",
                &pairs.into_iter().collect(),
                &[ipv4],
                8009,
            )
            .unwrap()
        };
        // What the device advertises while the receiver runs, in either receiver.
        assert!(device(Some("Jellyfin")).runs_receiver());
        assert!(device(Some("jellyfin (unstable)")).runs_receiver());
        // The idle screen, another app, or nothing said at all.
        assert!(!device(Some("Backdrop")).runs_receiver());
        assert!(!device(Some("YouTube")).runs_receiver());
        assert!(!device(None).runs_receiver());
    }
}
