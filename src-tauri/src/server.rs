use crate::models::{
    AppState, OfferRequest, OfferResponse, SelfInfo, TransferDirection, TransferItem,
    TransferStatus,
};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

#[derive(Clone)]
struct ServerContext {
    state: Arc<AppState>,
    app: AppHandle,
}

#[derive(Debug, Deserialize)]
struct BridgeQuery {
    t: Option<String>,
    cid: Option<String>,
    ids: Option<String>,
}

async fn is_authorized(state: &Arc<AppState>, q: &BridgeQuery) -> bool {
    let expected = state.mobile_token.read().await;
    matches!(q.t.as_deref(), Some(tok) if tok == expected.as_str())
}

const DEFAULT_PORT: u16 = 53317;

pub fn start_server(state: Arc<AppState>, app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT))).await {
            Ok(l) => l,
            Err(_) => TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], 0)))
                .await
                .expect("Failed to bind LocalFTP HTTP server"),
        };

        let local_port = listener.local_addr().map(|a| a.port()).unwrap_or(DEFAULT_PORT);
        *state.port.write().await = local_port;

        let ctx = ServerContext { state, app };

        let router = Router::new()
            .route("/", get(mobile_portal_html))
            .route("/api/info", get(get_info))
            .route("/api/offer", post(handle_offer))
            .route("/api/upload/{transfer_id}", post(handle_upload))
            .route("/api/shared", get(list_shared))
            .route("/api/shared/download-zip", get(download_shared_zip))
            .route("/api/shared/{file_id}/download", get(download_shared))
            .route("/api/shared/{file_id}/preview", get(preview_shared))
            .route("/api/transfers/{transfer_id}/preview", get(preview_transfer))
            .route("/api/mobile-sent", get(list_mobile_sent))
            .route("/api/mobile-upload", post(handle_mobile_upload))
            .layer(DefaultBodyLimit::disable())
            .with_state(ctx);

        if let Err(e) = axum::serve(listener, router).await {
            eprintln!("LocalFTP HTTP server error: {}", e);
        }
    });
}

async fn get_info(State(ctx): State<ServerContext>) -> Json<SelfInfo> {
    let ip = local_ip_address::local_ip()
        .map(|i| i.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let mobile_token = ctx.state.mobile_token.read().await.clone();
    Json(SelfInfo {
        id: ctx.state.self_id.clone(),
        name: ctx.state.device_name.read().await.clone(),
        os: ctx.state.os_name.clone(),
        ip,
        port: *ctx.state.port.read().await,
        save_dir: ctx.state.save_dir.read().await.to_string_lossy().to_string(),
        mobile_token,
    })
}

async fn handle_offer(
    State(ctx): State<ServerContext>,
    Json(req): Json<OfferRequest>,
) -> Json<OfferResponse> {
    let item = TransferItem {
        id: req.transfer_id.clone(),
        file_name: req.file_name.clone(),
        file_size: req.file_size,
        bytes_transferred: 0,
        peer_name: req.sender_name.clone(),
        peer_ip: String::new(),
        direction: TransferDirection::Incoming,
        status: TransferStatus::Transferring,
        save_path: None,
        error: None,
    };

    {
        let mut transfers = ctx.state.transfers.write().await;
        transfers.insert(0, item);
    }
    emit_transfers(&ctx.state, &ctx.app).await;

    Json(OfferResponse { accepted: true })
}

async fn handle_upload(
    State(ctx): State<ServerContext>,
    Path(transfer_id): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let save_dir = ctx.state.save_dir.read().await.clone();
    let _ = tokio::fs::create_dir_all(&save_dir).await;

    while let Ok(Some(mut field)) = multipart.next_field().await {
        let raw_name = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("file-{}", transfer_id));
        let dest_path = unique_save_path(&save_dir, &raw_name);

        let mut file = match tokio::fs::File::create(&dest_path).await {
            Ok(f) => f,
            Err(e) => {
                update_transfer_status(
                    &ctx.state,
                    &transfer_id,
                    TransferStatus::Failed,
                    Some(e.to_string()),
                )
                .await;
                emit_transfers(&ctx.state, &ctx.app).await;
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
            }
        };

        update_transfer_status(&ctx.state, &transfer_id, TransferStatus::Transferring, None).await;

        let mut written: u64 = 0;
        let mut last_emit = std::time::Instant::now();

        while let Ok(Some(chunk)) = field.chunk().await {
            if let Err(e) = file.write_all(&chunk).await {
                update_transfer_status(
                    &ctx.state,
                    &transfer_id,
                    TransferStatus::Failed,
                    Some(e.to_string()),
                )
                .await;
                emit_transfers(&ctx.state, &ctx.app).await;
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
            }
            written += chunk.len() as u64;

            if last_emit.elapsed().as_millis() >= 120 {
                update_transfer_progress(&ctx.state, &transfer_id, written).await;
                emit_transfers(&ctx.state, &ctx.app).await;
                last_emit = std::time::Instant::now();
            }
        }

        let _ = file.flush().await;
        {
            let mut transfers = ctx.state.transfers.write().await;
            if let Some(t) = transfers.iter_mut().find(|t| t.id == transfer_id) {
                t.bytes_transferred = written;
                t.status = TransferStatus::Completed;
                t.save_path = Some(dest_path.to_string_lossy().to_string());
            }
        }
        emit_transfers(&ctx.state, &ctx.app).await;
        return (StatusCode::OK, "OK".to_string());
    }

    (StatusCode::BAD_REQUEST, "No file field provided".to_string())
}

async fn list_shared(
    State(ctx): State<ServerContext>,
    Query(q): Query<BridgeQuery>,
) -> impl IntoResponse {
    if !is_authorized(&ctx.state, &q).await {
        return (StatusCode::UNAUTHORIZED, "Invalid or expired QR token").into_response();
    }
    let files = ctx.state.shared_files.read().await.clone();
    Json(files).into_response()
}

async fn list_mobile_sent(
    State(ctx): State<ServerContext>,
    Query(q): Query<BridgeQuery>,
) -> impl IntoResponse {
    if !is_authorized(&ctx.state, &q).await {
        return (StatusCode::UNAUTHORIZED, "Invalid or expired QR token").into_response();
    }
    let cid = q.cid.unwrap_or_else(|| "default".to_string());
    let target_tag = format!("mobile:{}", cid);
    let transfers = ctx.state.transfers.read().await;
    let sent: Vec<TransferItem> = transfers
        .iter()
        .filter(|t| t.peer_ip == target_tag)
        .cloned()
        .collect();
    Json(sent).into_response()
}

async fn download_shared(
    State(ctx): State<ServerContext>,
    Path(file_id): Path<String>,
    Query(q): Query<BridgeQuery>,
) -> impl IntoResponse {
    if !is_authorized(&ctx.state, &q).await {
        return (StatusCode::UNAUTHORIZED, "Invalid or expired QR token").into_response();
    }
    let shared = {
        let files = ctx.state.shared_files.read().await;
        files.iter().find(|f| f.id == file_id).cloned()
    };

    let Some(shared_file) = shared else {
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    };

    match tokio::fs::File::open(&shared_file.path).await {
        Ok(file) => {
            let stream = ReaderStream::new(file);
            let body = Body::from_stream(stream);
            let disposition = format!("attachment; filename=\"{}\"", shared_file.name.replace('"', "_"));
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                    (header::CONTENT_DISPOSITION, disposition),
                ],
                body,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Cannot open file").into_response(),
    }
}

async fn download_shared_zip(
    State(ctx): State<ServerContext>,
    Query(q): Query<BridgeQuery>,
) -> impl IntoResponse {
    if !is_authorized(&ctx.state, &q).await {
        return (StatusCode::UNAUTHORIZED, "Invalid or expired QR token").into_response();
    }

    let selected_ids: Vec<String> = q
        .ids
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let files = {
        let all = ctx.state.shared_files.read().await;
        if selected_ids.is_empty() {
            all.clone()
        } else {
            all.iter()
                .filter(|f| selected_ids.contains(&f.id))
                .cloned()
                .collect()
        }
    };

    if files.is_empty() {
        return (StatusCode::NOT_FOUND, "No files to download").into_response();
    }

    let temp_zip = std::env::temp_dir().join(format!("localftp-{}.zip", Uuid::new_v4()));
    let temp_zip_clone = temp_zip.clone();

    let zip_res = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let out_file = std::fs::File::create(&temp_zip_clone).map_err(|e| e.to_string())?;
        let mut writer = zip::ZipWriter::new(out_file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);

        let mut used_names = std::collections::HashSet::new();
        for f in files {
            let Ok(mut src) = std::fs::File::open(&f.path) else {
                continue;
            };
            let mut entry_name = f.name.clone();
            if !used_names.insert(entry_name.clone()) {
                entry_name = format!("{}-{}", &f.id[..4.min(f.id.len())], f.name);
                used_names.insert(entry_name.clone());
            }
            writer
                .start_file(entry_name, options)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut src, &mut writer).map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await;

    match zip_res {
        Ok(Ok(())) => match tokio::fs::File::open(&temp_zip).await {
            Ok(file) => {
                let stream = ReaderStream::new(file);
                let body = Body::from_stream(stream);
                // Clean up temp file after a short delay once opened
                let cleanup_path = temp_zip.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                    let _ = tokio::fs::remove_file(cleanup_path).await;
                });
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "application/zip".to_string()),
                        (
                            header::CONTENT_DISPOSITION,
                            "attachment; filename=\"LocalFTP-files.zip\"".to_string(),
                        ),
                    ],
                    body,
                )
                    .into_response()
            }
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read zip").into_response(),
        },
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create zip").into_response(),
    }
}

fn content_type_for_name(name: &str) -> &'static str {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

async fn preview_shared(
    State(ctx): State<ServerContext>,
    Path(file_id): Path<String>,
    Query(q): Query<BridgeQuery>,
) -> impl IntoResponse {
    if !is_authorized(&ctx.state, &q).await {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let shared = {
        let files = ctx.state.shared_files.read().await;
        files.iter().find(|f| f.id == file_id).cloned()
    };

    let Some(shared_file) = shared else {
        return (StatusCode::NOT_FOUND, "File not found").into_response();
    };

    match tokio::fs::File::open(&shared_file.path).await {
        Ok(file) => {
            let stream = ReaderStream::new(file);
            let body = Body::from_stream(stream);
            let mime = content_type_for_name(&shared_file.name);
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.to_string()),
                    (header::CONTENT_DISPOSITION, "inline".to_string()),
                ],
                body,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Cannot open file").into_response(),
    }
}

async fn preview_transfer(
    State(ctx): State<ServerContext>,
    Path(transfer_id): Path<String>,
    Query(q): Query<BridgeQuery>,
) -> impl IntoResponse {
    if !is_authorized(&ctx.state, &q).await {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let item = {
        let transfers = ctx.state.transfers.read().await;
        transfers.iter().find(|t| t.id == transfer_id).cloned()
    };

    let Some(transfer) = item else {
        return (StatusCode::NOT_FOUND, "Transfer not found").into_response();
    };
    let Some(path) = transfer.save_path else {
        return (StatusCode::NOT_FOUND, "No local file path").into_response();
    };

    match tokio::fs::File::open(&path).await {
        Ok(file) => {
            let stream = ReaderStream::new(file);
            let body = Body::from_stream(stream);
            let mime = content_type_for_name(&transfer.file_name);
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.to_string()),
                    (header::CONTENT_DISPOSITION, "inline".to_string()),
                ],
                body,
            )
                .into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Cannot open file").into_response(),
    }
}

async fn handle_mobile_upload(
    State(ctx): State<ServerContext>,
    Query(q): Query<BridgeQuery>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    if !is_authorized(&ctx.state, &q).await {
        return (StatusCode::UNAUTHORIZED, "Invalid or expired QR token".to_string());
    }
    let cid = q.cid.unwrap_or_else(|| "default".to_string());
    let peer_tag = format!("mobile:{}", cid);
    let save_dir = ctx.state.save_dir.read().await.clone();
    let _ = tokio::fs::create_dir_all(&save_dir).await;
    let mut saved_count = 0usize;

    while let Ok(Some(mut field)) = multipart.next_field().await {
        let raw_name = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("mobile-upload-{}", Uuid::new_v4()));
        let dest_path = unique_save_path(&save_dir, &raw_name);
        let transfer_id = Uuid::new_v4().to_string();

        let item = TransferItem {
            id: transfer_id.clone(),
            file_name: raw_name.clone(),
            file_size: 0,
            bytes_transferred: 0,
            peer_name: "Mobile Browser".to_string(),
            peer_ip: peer_tag.clone(),
            direction: TransferDirection::Incoming,
            status: TransferStatus::Transferring,
            save_path: None,
            error: None,
        };

        {
            let mut transfers = ctx.state.transfers.write().await;
            transfers.insert(0, item);
        }
        emit_transfers(&ctx.state, &ctx.app).await;

        let mut file = match tokio::fs::File::create(&dest_path).await {
            Ok(f) => f,
            Err(e) => {
                update_transfer_status(
                    &ctx.state,
                    &transfer_id,
                    TransferStatus::Failed,
                    Some(e.to_string()),
                )
                .await;
                emit_transfers(&ctx.state, &ctx.app).await;
                continue;
            }
        };

        let mut written: u64 = 0;
        while let Ok(Some(chunk)) = field.chunk().await {
            if file.write_all(&chunk).await.is_err() {
                break;
            }
            written += chunk.len() as u64;
        }
        let _ = file.flush().await;

        {
            let mut transfers = ctx.state.transfers.write().await;
            if let Some(t) = transfers.iter_mut().find(|t| t.id == transfer_id) {
                t.file_size = written;
                t.bytes_transferred = written;
                t.status = TransferStatus::Completed;
                t.save_path = Some(dest_path.to_string_lossy().to_string());
            }
        }
        emit_transfers(&ctx.state, &ctx.app).await;
        saved_count += 1;
    }

    if saved_count > 0 {
        (StatusCode::OK, format!("Uploaded {} file(s)", saved_count))
    } else {
        (StatusCode::BAD_REQUEST, "No files uploaded".to_string())
    }
}

fn unique_save_path(dir: &PathBuf, raw_name: &str) -> PathBuf {
    let safe_name = std::path::Path::new(raw_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("received-file");

    let candidate = dir.join(safe_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = std::path::Path::new(safe_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = std::path::Path::new(safe_name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();

    for i in 1..1000 {
        let numbered = dir.join(format!("{} ({}){}", stem, i, ext));
        if !numbered.exists() {
            return numbered;
        }
    }
    dir.join(format!("{}-{}{}", stem, Uuid::new_v4(), ext))
}

async fn update_transfer_progress(state: &Arc<AppState>, id: &str, bytes: u64) {
    let mut transfers = state.transfers.write().await;
    if let Some(t) = transfers.iter_mut().find(|t| t.id == id) {
        t.bytes_transferred = bytes;
    }
}

async fn update_transfer_status(
    state: &Arc<AppState>,
    id: &str,
    status: TransferStatus,
    error: Option<String>,
) {
    let mut transfers = state.transfers.write().await;
    if let Some(t) = transfers.iter_mut().find(|t| t.id == id) {
        t.status = status;
        if let Some(err) = error {
            t.error = Some(err);
        }
    }
}

pub async fn emit_transfers(state: &Arc<AppState>, app: &AppHandle) {
    let list = state.transfers.read().await.clone();
    let _ = app.emit("transfers-updated", list);
}

async fn mobile_portal_html(
    State(ctx): State<ServerContext>,
    Query(q): Query<BridgeQuery>,
) -> impl IntoResponse {
    if !is_authorized(&ctx.state, &q).await {
        return (
            StatusCode::UNAUTHORIZED,
            Html("<!doctype html><html><body style='font-family:-apple-system,sans-serif;background:#0f1115;color:#f4f4f5;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;text-align:center;padding:24px'><div><h2 style='margin:0 0 8px'>Access Denied</h2><p style='opacity:0.65;font-size:14px;margin:0'>Scan the current QR code in LocalFTP to connect.</p></div></body></html>"),
        )
            .into_response();
    }
    Html(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>LocalFTP Mobile Bridge</title>
  <style>
    * { box-sizing: border-box; }
    :root { color-scheme: light dark; font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", sans-serif; }
    body { margin: 0; padding: 0; width: 100%; min-height: 100vh; background: #0f1115; color: #f4f4f5; }
    @media (prefers-color-scheme: light) { body { background: #f5f5f7; color: #18181b; } .card { background: #ffffff !important; border-color: rgba(0,0,0,0.08) !important; } }
    .wrap { width: 100%; max-width: 560px; margin: 0 auto; padding: 22px 16px 36px; }
    .grid { display: flex; flex-direction: column; gap: 16px; }
    .col { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
    @media (min-width: 768px) {
      .wrap { max-width: 1000px; padding: 34px 30px 48px; }
      .grid { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1.15fr); gap: 28px; align-items: start; }
      .col { gap: 24px; }
    }
    h1 { font-size: 20px; font-weight: 600; margin: 0 0 4px; letter-spacing: -0.02em; }
    p.sub { font-size: 13px; opacity: 0.65; margin: 0 0 20px; }
    .card { background: rgba(255,255,255,0.06); border: 1px solid rgba(255,255,255,0.1); border-radius: 16px; padding: 16px; min-width: 0; }
    .label { font-size: 12px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em; opacity: 0.6; margin-bottom: 12px; }
    .btn { display: inline-flex; align-items: center; justify-content: center; width: 100%; padding: 12px 16px; border-radius: 12px; border: none; background: #3b82f6; color: #fff; font-size: 15px; font-weight: 600; cursor: pointer; }
    .sent-badge { padding: 4px 10px; border-radius: 999px; background: rgba(16,185,129,0.16); color: #34d399; font-size: 11px; font-weight: 600; flex-shrink: 0; text-transform: capitalize; }
    .batch-bar { display: flex; gap: 8px; margin-bottom: 12px; flex-wrap: wrap; }
    .batch-btn { flex: 1; min-width: 120px; padding: 9px 12px; border-radius: 10px; border: none; font-size: 13px; font-weight: 600; cursor: pointer; background: rgba(59,130,246,0.16); color: #60a5fa; }
    .batch-btn.primary { background: #3b82f6; color: #fff; }
    .batch-btn:disabled { opacity: 0.4; cursor: default; }
    .chk { width: 18px; height: 18px; accent-color: #3b82f6; flex-shrink: 0; cursor: pointer; }
    .file-row { display: flex; align-items: center; justify-content: space-between; padding: 10px 0; border-bottom: 1px solid rgba(128,128,128,0.15); gap: 12px; }
    .file-row:last-child { border-bottom: none; }
    .file-left { display: flex; align-items: center; gap: 12px; min-width: 0; flex: 1; }
    .thumb { width: 50px; height: 50px; border-radius: 10px; object-fit: cover; flex-shrink: 0; background: rgba(128,128,128,0.15); cursor: pointer; }
    .ext-badge { width: 50px; height: 50px; border-radius: 10px; display: flex; align-items: center; justify-content: center; font-size: 11px; font-weight: 700; letter-spacing: 0.04em; background: rgba(59,130,246,0.14); color: #60a5fa; flex-shrink: 0; }
    .file-meta { min-width: 0; }
    .file-name { font-size: 14px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .file-size { font-size: 12px; opacity: 0.6; }
    .dl { padding: 7px 14px; border-radius: 9px; background: rgba(59,130,246,0.16); color: #60a5fa; text-decoration: none; font-size: 13px; font-weight: 600; flex-shrink: 0; }
    #status { font-size: 13px; margin-top: 10px; opacity: 0.8; text-align: center; }
    #lightbox { position: fixed; inset: 0; background: rgba(0,0,0,0.86); display: none; align-items: center; justify-content: center; padding: 20px; z-index: 50; }
    #lightbox img { max-width: 100%; max-height: 88vh; border-radius: 12px; }
  </style>
</head>
<body>
  <div class="wrap">
    <h1 id="deviceTitle">LocalFTP</h1>
    <p class="sub">Direct local transfer over Wi-Fi / Hotspot</p>
    <div class="grid">
      <div class="col">
        <div class="card">
          <div class="label">Send to Computer</div>
          <input id="picker" type="file" multiple style="display:none" />
          <button class="btn" onclick="document.getElementById('picker').click()">Choose Photos or Files</button>
          <div id="status"></div>
        </div>
        <div class="card">
          <div class="label">Sent to Computer</div>
          <div id="sentList"><div style="font-size:13px;opacity:0.6">No files sent yet.</div></div>
        </div>
      </div>
      <div class="col">
        <div class="card">
          <div class="label">Shared from Computer</div>
          <div id="batchBar" class="batch-bar" style="display:none">
            <button id="dlSelectedBtn" class="batch-btn" onclick="downloadSelected()" disabled>Download Selected</button>
            <button id="dlAllBtn" class="batch-btn primary" onclick="downloadAll()">Download All</button>
          </div>
          <div id="sharedList"><div style="font-size:13px;opacity:0.6">Loading...</div></div>
        </div>
      </div>
    </div>
  </div>
  <div id="lightbox" onclick="this.style.display='none'"><img id="lightboxImg" alt="Preview" /></div>
  <script>
    const fmt = b => b < 1024 ? b + ' B' : b < 1048576 ? (b/1024).toFixed(1) + ' KB' : (b/1048576).toFixed(1) + ' MB';
    const isImg = name => /\.(png|jpe?g|gif|webp|svg|bmp)$/i.test(name);
    const getExt = name => { const p = name.split('.'); return p.length > 1 ? p.pop().slice(0,4).toUpperCase() : 'FILE'; };
    const token = new URLSearchParams(location.search).get('t') || '';
    let cid = localStorage.getItem('localftp-cid');
    if (!cid) {
      cid = (crypto.randomUUID ? crypto.randomUUID() : Math.random().toString(36).slice(2) + Date.now().toString(36));
      localStorage.setItem('localftp-cid', cid);
    }
    const authQ = `t=${encodeURIComponent(token)}&cid=${encodeURIComponent(cid)}`;
    function openPreview(url) {
      document.getElementById('lightboxImg').src = url;
      document.getElementById('lightbox').style.display = 'flex';
    }
    const selectedIds = new Set();
    let currentShared = [];
    function updateBatchBar() {
      const bar = document.getElementById('batchBar');
      const selBtn = document.getElementById('dlSelectedBtn');
      if (!currentShared.length) { bar.style.display = 'none'; return; }
      bar.style.display = 'flex';
      const count = selectedIds.size;
      selBtn.disabled = count === 0;
      selBtn.textContent = count > 0 ? `Download Selected (${count})` : 'Download Selected';
    }
    function toggleSelect(id, checked) {
      if (checked) selectedIds.add(id); else selectedIds.delete(id);
      updateBatchBar();
    }
    function downloadSelected() {
      const ids = Array.from(selectedIds);
      if (!ids.length) return;
      if (ids.length === 1) {
        location.href = `/api/shared/${ids[0]}/download?${authQ}`;
        return;
      }
      location.href = `/api/shared/download-zip?${authQ}&ids=${encodeURIComponent(ids.join(','))}`;
    }
    function downloadAll() {
      if (!currentShared.length) return;
      if (currentShared.length === 1) {
        location.href = `/api/shared/${currentShared[0].id}/download?${authQ}`;
        return;
      }
      location.href = `/api/shared/download-zip?${authQ}`;
    }
    fetch('/api/info').then(r => r.json()).then(d => {
      document.getElementById('deviceTitle').textContent = 'LocalFTP — ' + d.name;
    }).catch(() => {});
    async function loadShared() {
      try {
        const res = await fetch('/api/shared?' + authQ);
        if (!res.ok) return;
        const items = await res.json();
        currentShared = items;
        const validIds = new Set(items.map(i => i.id));
        for (const id of Array.from(selectedIds)) { if (!validIds.has(id)) selectedIds.delete(id); }
        updateBatchBar();
        const box = document.getElementById('sharedList');
        if (!items.length) { box.innerHTML = '<div style="font-size:13px;opacity:0.6">No files shared yet.</div>'; return; }
        box.innerHTML = items.map(f => {
          const previewUrl = `/api/shared/${f.id}/preview?${authQ}`;
          const checked = selectedIds.has(f.id) ? 'checked' : '';
          const thumb = isImg(f.name)
            ? `<img class="thumb" src="${previewUrl}" alt="${f.name}" onclick="openPreview('${previewUrl}')" />`
            : `<div class="ext-badge">${getExt(f.name)}</div>`;
          return `<div class="file-row"><div class="file-left"><input type="checkbox" class="chk" ${checked} onchange="toggleSelect('${f.id}', this.checked)" />${thumb}<div class="file-meta"><div class="file-name">${f.name}</div><div class="file-size">${fmt(f.size)}</div></div></div><a class="dl" href="/api/shared/${f.id}/download?${authQ}">Download</a></div>`;
        }).join('');
      } catch {}
    }
    async function loadSent() {
      try {
        const res = await fetch('/api/mobile-sent?' + authQ);
        if (!res.ok) return;
        const items = await res.json();
        const box = document.getElementById('sentList');
        if (!items.length) { box.innerHTML = '<div style="font-size:13px;opacity:0.6">No files sent yet.</div>'; return; }
        box.innerHTML = items.map(t => {
          const previewUrl = `/api/transfers/${t.id}/preview?${authQ}`;
          const thumb = (t.save_path && isImg(t.file_name))
            ? `<img class="thumb" src="${previewUrl}" alt="${t.file_name}" onclick="openPreview('${previewUrl}')" />`
            : `<div class="ext-badge">${getExt(t.file_name)}</div>`;
          return `<div class="file-row"><div class="file-left">${thumb}<div class="file-meta"><div class="file-name">${t.file_name}</div><div class="file-size">${fmt(t.file_size)}</div></div></div><span class="sent-badge">${t.status}</span></div>`;
        }).join('');
      } catch {}
    }
    loadShared();
    loadSent();
    setInterval(() => { loadShared(); loadSent(); }, 3000);
    document.getElementById('picker').addEventListener('change', async (e) => {
      const files = e.target.files;
      if (!files.length) return;
      const status = document.getElementById('status');
      status.textContent = 'Uploading ' + files.length + ' file(s)...';
      const fd = new FormData();
      for (const f of files) fd.append('file', f, f.name);
      const res = await fetch('/api/mobile-upload?' + authQ, {
        method: 'POST',
        body: fd
      });
      status.textContent = res.ok ? 'Sent successfully!' : 'Upload failed';
      e.target.value = '';
      loadSent();
    });
  </script>
</body>
</html>"#,
    ).into_response()
}
