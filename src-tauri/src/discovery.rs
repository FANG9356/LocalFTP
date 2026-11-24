use crate::models::{AppState, PeerInfo};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const SERVICE_TYPE: &str = "_localftp._tcp.local.";

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sanitize_hostname(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "localftp-device".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn start_discovery(state: Arc<AppState>, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Wait briefly until the HTTP server binds its port
        let port = loop {
            let p = *state.port.read().await;
            if p > 0 {
                break p;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };

        let mdns = match ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to start mDNS daemon: {}", e);
                return;
            }
        };

        let device_name = state.device_name.read().await.clone();
        let instance_name = format!("localftp-{}", &state.self_id[..8]);
        let host_name = format!("{}-{}.local.", sanitize_hostname(&device_name), &state.self_id[..4]);

        let tls_flag = if state.https_enabled { "1" } else { "0" };
        let properties = [
            ("id", state.self_id.as_str()),
            ("name", device_name.as_str()),
            ("os", state.os_name.as_str()),
            ("tls", tls_flag),
        ];

        let ip_str = local_ip_address::local_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_default();

        if let Ok(service_info) = ServiceInfo::new(
            SERVICE_TYPE,
            &instance_name,
            &host_name,
            &ip_str,
            port,
            &properties[..],
        ) {
            let service_info = service_info.enable_addr_auto();
            if let Err(e) = mdns.register(service_info) {
                eprintln!("Failed to register mDNS service: {}", e);
            }
        }

        let receiver = match mdns.browse(SERVICE_TYPE) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to browse mDNS service: {}", e);
                return;
            }
        };

        loop {
            match receiver.recv_async().await {
                Ok(event) => match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let peer_id = info
                            .get_property_val_str("id")
                            .unwrap_or_default()
                            .to_string();

                        if peer_id.is_empty() || peer_id == state.self_id {
                            continue;
                        }

                        let peer_name = info
                            .get_property_val_str("name")
                            .unwrap_or("Unknown Device")
                            .to_string();
                        let peer_os = info
                            .get_property_val_str("os")
                            .unwrap_or("Unknown")
                            .to_string();
                        let peer_https = info
                            .get_property_val_str("tls")
                            .map(|v| v != "0")
                            .unwrap_or(true);

                        let addrs = info.get_addresses();
                        let ip = addrs
                            .iter()
                            .find(|a| a.is_ipv4())
                            .or_else(|| addrs.iter().next())
                            .map(|a| a.to_string())
                            .unwrap_or_default();

                        if ip.is_empty() {
                            continue;
                        }

                        let peer = PeerInfo {
                            id: peer_id.clone(),
                            name: peer_name,
                            os: peer_os,
                            ip,
                            port: info.get_port(),
                            https_enabled: peer_https,
                            last_seen: now_secs(),
                        };

                        let mut peers = state.peers.write().await;
                        peers.insert(peer_id, peer);
                        let list: Vec<PeerInfo> = peers.values().cloned().collect();
                        drop(peers);

                        let _ = app.emit("peers-updated", list);
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        // Match by short id prefix in instance name if present
                        let mut peers = state.peers.write().await;
                        let before = peers.len();
                        peers.retain(|id, _| !fullname.contains(&id[..8.min(id.len())]));
                        if peers.len() != before {
                            let list: Vec<PeerInfo> = peers.values().cloned().collect();
                            drop(peers);
                            let _ = app.emit("peers-updated", list);
                        }
                    }
                    _ => {}
                },
                Err(e) => {
                    eprintln!("mDNS receiver error: {}", e);
                    break;
                }
            }
        }
    });
}
