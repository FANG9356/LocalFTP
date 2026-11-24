mod discovery;
mod models;
mod server;

use models::{
    AppState, OfferRequest, OfferResponse, PeerInfo, SelfInfo, SharedFile, TransferDirection,
    TransferItem, TransferStatus,
};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

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
        save_dir: state.save_dir.read().await.to_string_lossy().to_string(),
        mobile_token: state.mobile_token.read().await.clone(),
    })
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
        let client = reqwest::Client::new();
        let base_url = format!("http://{}:{}", peer.ip, peer.port);

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
            let stream = tokio_util::io::ReaderStream::new(file);
            let body = reqwest::Body::wrap_stream(stream);
            let part = reqwest::multipart::Part::stream_with_length(body, file_size)
                .file_name(file_name);
            let form = reqwest::multipart::Form::new().part("file", part);

            let upload_res = client
                .post(format!("{}/api/upload/{}", base_url, transfer_id))
                .multipart(form)
                .send()
                .await;

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
    let state = AppState::new(self_id, host_name, save_dir, mobile_token);

    let state_for_setup = Arc::clone(&state);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(move |app| {
            let handle = app.handle().clone();
            server::start_server(Arc::clone(&state_for_setup), handle.clone());
            discovery::start_discovery(Arc::clone(&state_for_setup), handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_self_info,
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
