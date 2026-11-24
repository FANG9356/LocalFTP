use crate::models::{AppState, PeerInfo};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::net::UdpSocket;

const SERVICE_TYPE: &str = "_localftp._tcp.local.";
const UDP_DISCOVERY_PORT: u16 = 53318;

#[derive(Debug, Serialize, Deserialize)]
struct UdpBeacon {
    id: String,
    name: String,
    os: String,
    port: u16,
    tls: bool,
}

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
    // UDP Broadcast fallback discovery alongside mDNS
    let udp_state = Arc::clone(&state);
    let udp_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let port = loop {
            let p = *udp_state.port.read().await;
            if p > 0 {
                break p;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };

        let Ok(sock) = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], UDP_DISCOVERY_PORT))).await
        else {
            return;
        };
        let _ = sock.set_broadcast(true);
        let sock = Arc::new(sock);

        let rx_sock = Arc::clone(&sock);
        let rx_state = Arc::clone(&udp_state);
        let rx_app = udp_app.clone();
        tauri::async_runtime::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok((len, src)) = rx_sock.recv_from(&mut buf).await {
                let Ok(beacon) = serde_json::from_slice::<UdpBeacon>(&buf[..len]) else {
                    continue;
                };
                if beacon.id.is_empty() || beacon.id == rx_state.self_id {
                    continue;
                }
                let peer = PeerInfo {
                    id: beacon.id.clone(),
                    name: beacon.name,
                    os: beacon.os,
                    ip: src.ip().to_string(),
                    port: beacon.port,
                    https_enabled: beacon.tls,
                    last_seen: now_secs(),
                };
                let mut peers = rx_state.peers.write().await;
                peers.insert(beacon.id, peer);
                let list: Vec<PeerInfo> = peers.values().cloned().collect();
                drop(peers);
                let _ = rx_app.emit("peers-updated", list);
            }
        });

        loop {
            let name = udp_state.device_name.read().await.clone();
            let beacon = UdpBeacon {
                id: udp_state.self_id.clone(),
                name,
                os: udp_state.os_name.clone(),
                port,
                tls: udp_state.https_enabled,
            };
            if let Ok(payload) = serde_json::to_vec(&beacon) {
                let _ = sock
                    .send_to(&payload, SocketAddr::from(([255, 255, 255, 255], UDP_DISCOVERY_PORT)))
                    .await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });

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
