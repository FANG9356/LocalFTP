mod discovery;
mod models;
mod server;

use models::{
    AppState, OfferRequest, OfferResponse, PeerInfo, SelfInfo, SharedFile, TransferDirection,
    TransferItem, TransferStatus,
};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncRead, ReadBuf};
use uuid::Uuid;

struct ProgressReader<R> {
    inner: R,
    sent: Arc<AtomicU64>,
}

impl<R: AsyncRead + Unpin> AsyncRead for ProgressReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &poll {
            let delta = buf.filled().len().saturating_sub(before) as u64;
            if delta > 0 {
                self.sent.fetch_add(delta, Ordering::Relaxed);
            }
        }
        poll
    }
}

fn settings_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            if exe_dir.file_name().and_then(|n| n.to_str()) == Some("MacOS") {
                if let Some(contents_dir) = exe_dir.parent() {
                    return contents_dir.join("Resources").join("settings.json");
                }
            }
            return exe_dir.join("settings.json");
        }
    }
    PathBuf::from("settings.json")
}

fn load_https_setting() -> bool {
    let path = settings_path();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return true;
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("https_enabled").and_then(|b| b.as_bool()))
        .unwrap_or(true)
}

fn save_https_setting(enabled: bool) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::json!({ "https_enabled": enabled });
    std::fs::write(path, serde_json::to_string_pretty(&json).unwrap_or_default())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_self_info(state: State<'_, Arc<AppState>>) -> Result<SelfInfo, String> {
    let ip = local_ip_address::local_ip()
        .map(|i| i.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    Ok(SelfInfo {
        id: state.self_id.clone(),
        name: state.device_name.read().await.clone(),
        os: state.os_name.clone(),
        ip,
        port: *state.port.read().await,
        local_port: *state.local_port.read().await,
        save_dir: state.save_dir.read().await.to_string_lossy().to_string(),
        mobile_token: state.mobile_token.read().await.clone(),
        auto_accept: *state.auto_accept.read().await,
        https_enabled: state.https_enabled,
    })
}

#[tauri::command]
async fn set_https_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    save_https_setting(enabled)?;
    #[cfg(target_os = "macos")]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(app_bundle) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            if app_bundle.extension().and_then(|e| e.to_str()) == Some("app") {
                let _ = std::process::Command::new("open")
                    .arg("-n")
                    .arg(app_bundle)
                    .spawn();
                app.exit(0);
                return Ok(());
            }
        }
    }
    app.restart();
}

#[tauri::command]
async fn set_auto_accept(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    *state.auto_accept.write().await = enabled;
    let _ = app.emit("self-updated", ());
    Ok(())
}

#[tauri::command]
async fn regenerate_mobile_token(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<String, String> {
    let new_token = Uuid::new_v4().simple().to_string()[..16].to_string();
    *state.mobile_token.write().await = new_token.clone();
    let _ = app.emit("self-updated", ());
    Ok(new_token)
}

#[tauri::command]
async fn get_peers(state: State<'_, Arc<AppState>>) -> Result<Vec<PeerInfo>, String> {
    Ok(state.peers.read().await.values().cloned().collect())
}

#[tauri::command]
async fn get_transfers(state: State<'_, Arc<AppState>>) -> Result<Vec<TransferItem>, String> {
    Ok(state.transfers.read().await.clone())
}

#[tauri::command]
async fn respond_to_offer(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    transfer_id: String,
    accept: bool,
) -> Result<(), String> {
    let tx = {
        let mut pending = state.pending_approvals.write().await;
        pending.remove(&transfer_id)
    };
    if let Some(sender) = tx {
        let _ = sender.send(accept);
    }
    if !accept {
        let mut transfers = state.transfers.write().await;
        if let Some(t) = transfers.iter_mut().find(|t| t.id == transfer_id) {
            t.status = TransferStatus::Rejected;
        }
        drop(transfers);
        server::emit_transfers(&state, &app).await;
    }
    Ok(())
}

#[tauri::command]
async fn send_files_to_peer(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    peer_id: String,
    file_paths: Vec<String>,
) -> Result<(), String> {
    let peer = {
        let peers = state.peers.read().await;
        peers
            .get(&peer_id)
            .cloned()
            .ok_or_else(|| "Peer is no longer available".to_string())?
    };

    let sender_name = state.device_name.read().await.clone();
    let shared_state = Arc::clone(&state);

    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        let scheme = if peer.https_enabled { "https" } else { "http" };
        let base_url = format!("{}://{}:{}", scheme, peer.ip, peer.port);

        for raw_path in file_paths {
            let path = PathBuf::from(&raw_path);
            let Ok(meta) = tokio::fs::metadata(&path).await else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            let file_size = meta.len();
            let transfer_id = Uuid::new_v4().to_string();

            let item = TransferItem {
                id: transfer_id.clone(),
                file_name: file_name.clone(),
                file_size,
                bytes_transferred: 0,
                peer_name: peer.name.clone(),
                peer_ip: peer.ip.clone(),
                direction: TransferDirection::Outgoing,
                status: TransferStatus::Pending,
                save_path: Some(raw_path.clone()),
                error: None,
            };

            {
                let mut transfers = shared_state.transfers.write().await;
                transfers.insert(0, item);
            }
            server::emit_transfers(&shared_state, &app).await;

            // 1. Send offer and wait for recipient approval
            let offer = OfferRequest {
                transfer_id: transfer_id.clone(),
                file_name: file_name.clone(),
                file_size,
                sender_name: sender_name.clone(),
            };

            let offer_res = client
                .post(format!("{}/api/offer", base_url))
                .json(&offer)
                .send()
                .await;

            let accepted = match offer_res {
                Ok(resp) if resp.status().is_success() => resp
                    .json::<OfferResponse>()
                    .await
                    .map(|r| r.accepted)
                    .unwrap_or(false),
                _ => false,
            };

            if !accepted {
                let mut transfers = shared_state.transfers.write().await;
                if let Some(t) = transfers.iter_mut().find(|t| t.id == transfer_id) {
                    t.status = TransferStatus::Rejected;
                }
                drop(transfers);
                server::emit_transfers(&shared_state, &app).await;
                continue;
            }

            // 2. Stream file to peer
            {
                let mut transfers = shared_state.transfers.write().await;
                if let Some(t) = transfers.iter_mut().find(|t| t.id == transfer_id) {
                    t.status = TransferStatus::Transferring;
                }
            }
            server::emit_transfers(&shared_state, &app).await;

            let Ok(file) = tokio::fs::File::open(&path).await else {
                continue;
            };
            let sent_counter = Arc::new(AtomicU64::new(0));
            let upload_done = Arc::new(AtomicBool::new(false));
            let reader = ProgressReader {
                inner: file,
                sent: Arc::clone(&sent_counter),
            };
            let stream = tokio_util::io::ReaderStream::new(reader);
            let body = reqwest::Body::wrap_stream(stream);
            let part = reqwest::multipart::Part::stream_with_length(body, file_size)
                .file_name(file_name);
            let form = reqwest::multipart::Form::new().part("file", part);

            let ticker_state = Arc::clone(&shared_state);
            let ticker_app = app.clone();
            let ticker_id = transfer_id.clone();
            let ticker_sent = Arc::clone(&sent_counter);
            let ticker_done = Arc::clone(&upload_done);
            let ticker = tauri::async_runtime::spawn(async move {
                while !ticker_done.load(Ordering::Relaxed) {
                    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                    let bytes = ticker_sent.load(Ordering::Relaxed).min(file_size);
                    {
                        let mut transfers = ticker_state.transfers.write().await;
                        if let Some(t) = transfers.iter_mut().find(|t| t.id == ticker_id) {
                            t.bytes_transferred = bytes;
                        }
                    }
                    server::emit_transfers(&ticker_state, &ticker_app).await;
                }
            });

            let upload_res = client
                .post(format!("{}/api/upload/{}", base_url, transfer_id))
                .multipart(form)
                .send()
                .await;

            upload_done.store(true, Ordering::Relaxed);
            let _ = ticker.await;

            let mut transfers = shared_state.transfers.write().await;
            if let Some(t) = transfers.iter_mut().find(|t| t.id == transfer_id) {
                match upload_res {
                    Ok(r) if r.status().is_success() => {
                        t.bytes_transferred = file_size;
                        t.status = TransferStatus::Completed;
                    }
                    Ok(r) => {
                        t.status = TransferStatus::Failed;
                        t.error = Some(format!("HTTP {}", r.status()));
                    }
                    Err(e) => {
                        t.status = TransferStatus::Failed;
                        t.error = Some(e.to_string());
                    }
                }
            }
            drop(transfers);
            server::emit_transfers(&shared_state, &app).await;
        }
    });

    Ok(())
}

#[tauri::command]
async fn share_files_for_mobile(
    state: State<'_, Arc<AppState>>,
    file_paths: Vec<String>,
) -> Result<Vec<SharedFile>, String> {
    let mut shared = state.shared_files.write().await;
    for raw_path in file_paths {
        let path = PathBuf::from(&raw_path);
        if let Ok(meta) = tokio::fs::metadata(&path).await {
            if meta.is_file() && !shared.iter().any(|f| f.path == raw_path) {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string();
                shared.push(SharedFile {
                    id: Uuid::new_v4().to_string(),
                    name,
                    size: meta.len(),
                    path: raw_path,
                });
            }
        }
    }
    Ok(shared.clone())
}

#[tauri::command]
async fn remove_shared_file(
    state: State<'_, Arc<AppState>>,
    file_id: String,
) -> Result<Vec<SharedFile>, String> {
    let mut shared = state.shared_files.write().await;
    shared.retain(|f| f.id != file_id);
    Ok(shared.clone())
}

#[tauri::command]
async fn get_shared_files(state: State<'_, Arc<AppState>>) -> Result<Vec<SharedFile>, String> {
    Ok(state.shared_files.read().await.clone())
}

#[tauri::command]
async fn set_save_dir(state: State<'_, Arc<AppState>>, path: String) -> Result<String, String> {
    let pb = PathBuf::from(&path);
    if !pb.is_dir() {
        return Err("Selected path is not a directory".to_string());
    }
    *state.save_dir.write().await = pb;
    Ok(path)
}

#[tauri::command]
async fn set_device_name(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    name: String,
) -> Result<(), String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err("Device name cannot be empty".to_string());
    }
    *state.device_name.write().await = trimmed;
    let _ = app.emit("self-updated", ());
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let host_name = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "My Device".to_string())
        .trim_end_matches(".local")
        .to_string();

    let save_dir = dirs::download_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("LocalFtp");
    let _ = std::fs::create_dir_all(&save_dir);
    let self_id = Uuid::new_v4().to_string();
    let mobile_token = Uuid::new_v4().simple().to_string()[..16].to_string();
    let https_enabled = load_https_setting();
    let state = AppState::new(self_id, host_name, save_dir, mobile_token, https_enabled);

    let state_for_setup = Arc::clone(&state);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(move |app| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.set_focus();
            }
            let handle = app.handle().clone();
            server::start_server(Arc::clone(&state_for_setup), handle.clone());
            discovery::start_discovery(Arc::clone(&state_for_setup), handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_self_info,
            set_auto_accept,
            set_https_enabled,
            regenerate_mobile_token,
            get_peers,
            get_transfers,
            respond_to_offer,
            send_files_to_peer,
            share_files_for_mobile,
            remove_shared_file,
            get_shared_files,
            set_save_dir,
            set_device_name,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LocalFTP");
}
