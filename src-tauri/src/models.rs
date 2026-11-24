use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: String,
    pub name: String,
    pub os: String,
    pub ip: String,
    pub port: u16,
    pub https_enabled: bool,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfInfo {
    pub id: String,
    pub name: String,
    pub os: String,
    pub ip: String,
    pub port: u16,
    pub local_port: u16,
    pub save_dir: String,
    pub watch_dir: Option<String>,
    pub mobile_token: String,
    pub auto_accept: bool,
    pub https_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TransferDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TransferStatus {
    Pending,
    Transferring,
    Completed,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferItem {
    pub id: String,
    pub file_name: String,
    pub file_size: u64,
    pub bytes_transferred: u64,
    pub peer_name: String,
    pub peer_ip: String,
    pub direction: TransferDirection,
    pub status: TransferStatus,
    pub save_path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedFile {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub path: String,
    #[serde(default)]
    pub from_watch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferRequest {
    pub transfer_id: String,
    pub file_name: String,
    pub file_size: u64,
    pub sender_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferResponse {
    pub accepted: bool,
}

pub struct AppState {
    pub self_id: String,
    pub device_name: RwLock<String>,
    pub os_name: String,
    pub port: RwLock<u16>,
    pub local_port: RwLock<u16>,
    pub save_dir: RwLock<PathBuf>,
    pub watch_dir: RwLock<Option<PathBuf>>,
    pub mobile_token: RwLock<String>,
    pub auto_accept: RwLock<bool>,
    pub https_enabled: bool,
    pub peers: RwLock<HashMap<String, PeerInfo>>,
    pub transfers: RwLock<Vec<TransferItem>>,
    pub pending_approvals: RwLock<HashMap<String, oneshot::Sender<bool>>>,
    pub shared_files: RwLock<Vec<SharedFile>>,
}

impl AppState {
    pub fn new(
        self_id: String,
        device_name: String,
        save_dir: PathBuf,
        mobile_token: String,
        https_enabled: bool,
    ) -> Arc<Self> {
        let os_name = match std::env::consts::OS {
            "macos" => "macOS",
            "windows" => "Windows",
            "linux" => "Linux",
            other => other,
        }
        .to_string();

        Arc::new(Self {
            self_id,
            device_name: RwLock::new(device_name),
            os_name,
            port: RwLock::new(0),
            local_port: RwLock::new(0),
            save_dir: RwLock::new(save_dir),
            watch_dir: RwLock::new(None),
            mobile_token: RwLock::new(mobile_token),
            auto_accept: RwLock::new(false),
            https_enabled,
            peers: RwLock::new(HashMap::new()),
            transfers: RwLock::new(Vec::new()),
            pending_approvals: RwLock::new(HashMap::new()),
            shared_files: RwLock::new(Vec::new()),
        })
    }
}
