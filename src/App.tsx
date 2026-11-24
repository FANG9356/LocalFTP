import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { QRCodeSVG } from "qrcode.react";
import {
  ArrowDownLeft,
  ArrowUpRight,
  Check,
  File as FileIcon,
  FileText,
  Film,
  FolderOpen,
  Laptop,
  Monitor,
  Moon,
  Music,
  Palette,
  QrCode,
  Radio,
  RefreshCw,
  Send,
  SlidersHorizontal,
  Smartphone,
  Sun,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { ACCENT_THEMES, formatBytes } from "./themes";
import type {
  AccentTheme,
  PeerInfo,
  SelfInfo,
  SharedFile,
  SurfaceMode,
  TransferItem,
} from "./types";

type TabKey = "nearby" | "mobile" | "transfers";

type PreviewKind = "image" | "video" | "audio" | "pdf" | "text";

function getPreviewKind(name: string): PreviewKind | null {
  if (/\.(png|jpe?g|gif|webp|svg|bmp|ico)$/i.test(name)) return "image";
  if (/\.(mp4|mov|webm|m4v)$/i.test(name)) return "video";
  if (/\.(mp3|wav|m4a|aac|ogg|flac)$/i.test(name)) return "audio";
  if (/\.pdf$/i.test(name)) return "pdf";
  if (/\.(txt|md|json|csv|log|xml|yaml|yml|toml|ini|sh|rs|ts|tsx|js|jsx|py|css|html|env)$/i.test(name)) {
    return "text";
  }
  return null;
}

interface PreviewTarget {
  url: string;
  name: string;
  kind: PreviewKind;
}

function formatNodeCode(id: string): string {
  const clean = id.replace(/[^a-zA-Z0-9]/g, "").slice(0, 4).toUpperCase();
  return `LF-${clean || "NODE"}`;
}

function OsIcon({ os, className }: { os: string; className?: string }) {
  const lower = os.toLowerCase();
  if (lower.includes("mac")) return <Laptop className={className} strokeWidth={1.6} />;
  if (lower.includes("win") || lower.includes("linux")) {
    return <Monitor className={className} strokeWidth={1.6} />;
  }
  return <Smartphone className={className} strokeWidth={1.6} />;
}

export default function App() {
  const [tab, setTab] = useState<TabKey>("nearby");
  const [surface, setSurface] = useState<SurfaceMode>(() => {
    return (localStorage.getItem("localftp-surface") as SurfaceMode) || "dark";
  });
  const [accentId, setAccentId] = useState<string>(() => {
    return localStorage.getItem("localftp-accent") || "ocean";
  });

  const [selfInfo, setSelfInfo] = useState<SelfInfo | null>(null);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [transfers, setTransfers] = useState<TransferItem[]>([]);
  const [sharedFiles, setSharedFiles] = useState<SharedFile[]>([]);

  const [isDragging, setIsDragging] = useState(false);
  const [queuedPaths, setQueuedPaths] = useState<string[] | null>(null);
  const [pendingOffer, setPendingOffer] = useState<TransferItem | null>(null);
  const [previewTarget, setPreviewTarget] = useState<PreviewTarget | null>(null);
  const [previewText, setPreviewText] = useState<string>("");

  const [showThemeMenu, setShowThemeMenu] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [editingName, setEditingName] = useState("");

  const accent: AccentTheme = useMemo(
    () => ACCENT_THEMES.find((t) => t.id === accentId) || ACCENT_THEMES[0],
    [accentId]
  );

  useEffect(() => {
    localStorage.setItem("localftp-surface", surface);
  }, [surface]);

  useEffect(() => {
    localStorage.setItem("localftp-accent", accentId);
  }, [accentId]);

  useEffect(() => {
    const loadInitial = async () => {
      try {
        const [info, initialPeers, initialTransfers, initialShared] = await Promise.all([
          invoke<SelfInfo>("get_self_info"),
          invoke<PeerInfo[]>("get_peers"),
          invoke<TransferItem[]>("get_transfers"),
          invoke<SharedFile[]>("get_shared_files"),
        ]);
        setSelfInfo(info);
        setEditingName(info.name);
        setPeers(initialPeers);
        setTransfers(initialTransfers);
        setSharedFiles(initialShared);
      } catch (err) {
        console.error("Failed to load initial state:", err);
      }
    };
    loadInitial();

    const unlistenPeers = listen<PeerInfo[]>("peers-updated", (e) => {
      setPeers(e.payload);
    });
    const unlistenTransfers = listen<TransferItem[]>("transfers-updated", (e) => {
      setTransfers(e.payload);
    });
    const unlistenOffer = listen<TransferItem>("transfer-offer", (e) => {
      setPendingOffer(e.payload);
    });
    const unlistenSelf = listen("self-updated", async () => {
      const info = await invoke<SelfInfo>("get_self_info");
      setSelfInfo(info);
    });

    let unlistenDrop: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "over" || event.payload.type === "enter") {
          setIsDragging(true);
        } else if (event.payload.type === "leave") {
          setIsDragging(false);
        } else if (event.payload.type === "drop") {
          setIsDragging(false);
          const paths = event.payload.paths;
          if (paths && paths.length > 0) {
            if (tab === "mobile") {
              invoke<SharedFile[]>("share_files_for_mobile", { filePaths: paths }).then(
                setSharedFiles
              );
            } else {
              setQueuedPaths(paths);
            }
          }
        }
      })
      .then((fn) => {
        unlistenDrop = fn;
      });

    return () => {
      unlistenPeers.then((f) => f());
      unlistenTransfers.then((f) => f());
      unlistenOffer.then((f) => f());
      unlistenSelf.then((f) => f());
      if (unlistenDrop) unlistenDrop();
    };
  }, [tab]);

  const isDark = surface === "dark";

  const shellClass = isDark
    ? "bg-zinc-950/55 text-zinc-100 border-white/10"
    : "bg-white/65 text-zinc-900 border-black/10";

  const glassCard = isDark
    ? "bg-white/[0.06] border border-white/12 shadow-[0_8px_24px_rgba(0,0,0,0.22)]"
    : "bg-white/75 border border-black/8 shadow-[0_8px_24px_rgba(0,0,0,0.06)]";

  const subtleText = isDark ? "text-zinc-400" : "text-zinc-500";

  const pickFilesForPeer = async (peer: PeerInfo) => {
    const selected = await openDialog({ multiple: true, directory: false });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    if (paths.length > 0) {
      await invoke("send_files_to_peer", { peerId: peer.id, filePaths: paths });
      setTab("transfers");
    }
  };

  const pickFilesGeneral = async () => {
    const selected = await openDialog({ multiple: true, directory: false });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    if (paths.length > 0) {
      setQueuedPaths(paths);
    }
  };

  const sendQueuedToPeer = async (peerId: string) => {
    if (!queuedPaths || queuedPaths.length === 0) return;
    await invoke("send_files_to_peer", { peerId, filePaths: queuedPaths });
    setQueuedPaths(null);
    setTab("transfers");
  };

  const shareQueuedToMobile = async () => {
    if (!queuedPaths || queuedPaths.length === 0) return;
    const updated = await invoke<SharedFile[]>("share_files_for_mobile", {
      filePaths: queuedPaths,
    });
    setSharedFiles(updated);
    setQueuedPaths(null);
    setTab("mobile");
  };

  const pickFilesForMobile = async () => {
    const selected = await openDialog({ multiple: true, directory: false });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    if (paths.length > 0) {
      const updated = await invoke<SharedFile[]>("share_files_for_mobile", {
        filePaths: paths,
      });
      setSharedFiles(updated);
    }
  };

  const openFilePreview = async (url: string, name: string, kind: PreviewKind) => {
    setPreviewTarget({ url, name, kind });
    if (kind === "text") {
      setPreviewText("Loading...");
      try {
        const res = await fetch(url);
        const text = await res.text();
        setPreviewText(text.slice(0, 100000));
      } catch {
        setPreviewText("Unable to load preview.");
      }
    }
  };

  const renderPreviewThumb = (url: string | null, name: string, sizeClass = "h-9 w-9") => {
    const kind = url ? getPreviewKind(name) : null;
    if (url && kind === "image") {
      return (
        <img
          src={url}
          alt={name}
          onClick={() => openFilePreview(url, name, "image")}
          className={`${sizeClass} rounded-lg object-cover shrink-0 cursor-pointer hover:opacity-90`}
        />
      );
    }
    if (url && kind) {
      const Icon = kind === "video" ? Film : kind === "audio" ? Music : FileText;
      return (
        <button
          onClick={() => openFilePreview(url, name, kind)}
          title={`Preview ${name}`}
          className={`${sizeClass} rounded-lg flex items-center justify-center shrink-0 cursor-pointer hover:opacity-85 transition-opacity`}
          style={{ background: accent.soft, color: accent.primary }}
        >
          <Icon className="h-4 w-4" />
        </button>
      );
    }
    return (
      <div
        className={`${sizeClass} rounded-lg flex items-center justify-center shrink-0`}
        style={{ background: accent.soft, color: accent.primary }}
      >
        <FileIcon className="h-4 w-4" />
      </div>
    );
  };

  const handleRespondOffer = async (accept: boolean) => {
    if (!pendingOffer) return;
    await invoke("respond_to_offer", { transferId: pendingOffer.id, accept });
    setPendingOffer(null);
    if (accept) setTab("transfers");
  };

  const handleToggleAutoAccept = async () => {
    if (!selfInfo) return;
    await invoke("set_auto_accept", { enabled: !selfInfo.auto_accept });
  };

  const handleResetMobileToken = async () => {
    await invoke("regenerate_mobile_token");
  };

  const handleChooseSaveDir = async () => {
    const dir = await openDialog({ directory: true, multiple: false });
    if (dir && typeof dir === "string") {
      await invoke("set_save_dir", { path: dir });
      const info = await invoke<SelfInfo>("get_self_info");
      setSelfInfo(info);
    }
  };

  const handleSaveDeviceName = async () => {
    if (!editingName.trim()) return;
    await invoke("set_device_name", { name: editingName.trim() });
    const info = await invoke<SelfInfo>("get_self_info");
    setSelfInfo(info);
    setShowSettings(false);
  };

  const activeTransfersCount = transfers.filter(
    (t) => t.status === "transferring" || t.status === "pending"
  ).length;

  const mobileUrl = selfInfo
    ? `https://${selfInfo.ip}:${selfInfo.port}/?t=${encodeURIComponent(selfInfo.mobile_token)}`
    : "";

  return (
    <div
      className={`relative h-screen w-screen flex flex-col overflow-hidden transition-colors duration-300 ${shellClass}`}
    >
      {/* Ambient theme glow */}
      <div
        className="pointer-events-none absolute inset-0 opacity-22 transition-all duration-500"
        style={{
          background: `radial-gradient(ellipse 70% 48% at 50% 0%, ${accent.primary} 0%, transparent 72%)`,
        }}
      />

      {/* macOS Overlay Titlebar */}
      <header
        data-tauri-drag-region
        className={`relative z-20 flex items-center justify-between h-13 px-4 pl-22 border-b ${
          isDark ? "border-white/8" : "border-black/6"
        }`}
      >
        <div data-tauri-drag-region className="flex items-center gap-2.5">
          <div
            className="h-6 w-6 rounded-lg flex items-center justify-center text-white shadow-sm"
            style={{ background: accent.primary }}
          >
            <Radio className="h-3.5 w-3.5" strokeWidth={2.2} />
          </div>
          <span className="text-[13px] font-semibold tracking-tight">LocalFTP</span>
          {selfInfo && (
            <span
              className={`hidden sm:inline-flex items-center gap-1.5 text-[11px] px-2 py-0.5 rounded-full ${
                isDark ? "bg-white/8 text-zinc-300" : "bg-black/5 text-zinc-600"
              }`}
            >
              <span
                className="h-1.5 w-1.5 rounded-full"
                style={{ background: accent.primary }}
              />
              {formatNodeCode(selfInfo.id)}
            </span>
          )}
        </div>

        {/* Segmented Navigation */}
        <nav
          className={`flex items-center gap-1 p-1 rounded-xl ${
            isDark ? "bg-white/[0.06]" : "bg-black/[0.05]"
          }`}
        >
          {(
            [
              { key: "nearby", label: "Nearby", icon: Radio },
              { key: "mobile", label: "Mobile QR", icon: QrCode },
              { key: "transfers", label: "Transfers", icon: ArrowUpRight },
            ] as const
          ).map((item) => {
            const Icon = item.icon;
            const active = tab === item.key;
            return (
              <button
                key={item.key}
                onClick={() => setTab(item.key)}
                className={`relative flex items-center gap-1.5 px-3 py-1 rounded-lg text-[12px] font-medium transition-all cursor-pointer ${
                  active
                    ? isDark
                      ? "bg-white/15 text-white shadow-sm"
                      : "bg-white text-zinc-900 shadow-sm"
                    : `${subtleText} hover:opacity-80`
                }`}
              >
                <Icon className="h-3.5 w-3.5" strokeWidth={1.8} />
                <span>{item.label}</span>
                {item.key === "transfers" && activeTransfersCount > 0 && (
                  <span
                    className="ml-0.5 px-1.5 py-0.2 text-[10px] rounded-full text-white font-semibold"
                    style={{ background: accent.primary }}
                  >
                    {activeTransfersCount}
                  </span>
                )}
              </button>
            );
          })}
        </nav>

        {/* Theme & Settings Controls */}
        <div className="relative flex items-center gap-1.5">
          <button
            onClick={() => setSurface(isDark ? "light" : "dark")}
            title="Toggle Light / Dark Frost"
            className={`p-1.5 rounded-lg transition-colors cursor-pointer ${
              isDark ? "hover:bg-white/10" : "hover:bg-black/6"
            }`}
          >
            {isDark ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </button>

          <button
            onClick={() => {
              setShowThemeMenu((v) => !v);
              setShowSettings(false);
            }}
            title="Theme Accent Color"
            className={`p-1.5 rounded-lg transition-colors cursor-pointer ${
              isDark ? "hover:bg-white/10" : "hover:bg-black/6"
            }`}
          >
            <Palette className="h-4 w-4" style={{ color: accent.primary }} />
          </button>

          <button
            onClick={() => {
              setShowSettings((v) => !v);
              setShowThemeMenu(false);
            }}
            title="Settings"
            className={`p-1.5 rounded-lg transition-colors cursor-pointer ${
              isDark ? "hover:bg-white/10" : "hover:bg-black/6"
            }`}
          >
            <SlidersHorizontal className="h-4 w-4" />
          </button>

          {/* Click-outside backdrop for Theme / Settings popovers */}
          {(showThemeMenu || showSettings) && (
            <div
              onClick={() => {
                setShowThemeMenu(false);
                setShowSettings(false);
              }}
              className="fixed inset-0 z-40"
            />
          )}

          {/* Accent Picker Popover */}
          {showThemeMenu && (
            <div
              className={`absolute right-0 top-10 w-52 rounded-2xl p-3 backdrop-blur-2xl z-50 ${glassCard}`}
            >
              <div className={`text-[11px] font-semibold uppercase tracking-wider mb-2.5 ${subtleText}`}>
                Theme Accent
              </div>
              <div className="space-y-1">
                {ACCENT_THEMES.map((t) => (
                  <button
                    key={t.id}
                    onClick={() => {
                      setAccentId(t.id);
                      setShowThemeMenu(false);
                    }}
                    className={`w-full flex items-center justify-between px-2.5 py-1.5 rounded-xl text-[12px] font-medium transition-colors cursor-pointer ${
                      accentId === t.id
                        ? isDark
                          ? "bg-white/12"
                          : "bg-black/6"
                        : isDark
                        ? "hover:bg-white/6"
                        : "hover:bg-black/4"
                    }`}
                  >
                    <span className="flex items-center gap-2.5">
                      <span
                        className="h-3.5 w-3.5 rounded-full"
                        style={{ background: t.swatch }}
                      />
                      {t.label}
                    </span>
                    {accentId === t.id && <Check className="h-3.5 w-3.5" />}
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* Settings Popover */}
          {showSettings && (
            <div
              className={`absolute right-0 top-10 w-72 rounded-2xl p-4 backdrop-blur-2xl z-50 ${glassCard}`}
            >
              <div className="flex items-center justify-between mb-3">
                <span className="text-[13px] font-semibold">Device Settings</span>
                <button
                  onClick={() => setShowSettings(false)}
                  className="opacity-60 hover:opacity-100 cursor-pointer"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>
              <div className="space-y-3">
                <div>
                  <label className={`block text-[11px] font-medium mb-1 ${subtleText}`}>
                    Device Name
                  </label>
                  <div className="flex gap-1.5">
                    <input
                      value={editingName}
                      onChange={(e) => setEditingName(e.target.value)}
                      className={`flex-1 px-2.5 py-1.5 rounded-lg text-[12px] outline-none border ${
                        isDark
                          ? "bg-white/8 border-white/12 text-white"
                          : "bg-white/80 border-black/10 text-zinc-900"
                      }`}
                    />
                    <button
                      onClick={handleSaveDeviceName}
                      className="px-2.5 py-1.5 rounded-lg text-[12px] font-medium text-white cursor-pointer"
                      style={{ background: accent.primary }}
                    >
                      Save
                    </button>
                  </div>
                </div>
                <div>
                  <label className={`block text-[11px] font-medium mb-1 ${subtleText}`}>
                    Downloads Folder
                  </label>
                  <button
                    onClick={handleChooseSaveDir}
                    className={`w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[12px] text-left border truncate cursor-pointer ${
                      isDark
                        ? "bg-white/6 border-white/10 hover:bg-white/10"
                        : "bg-black/4 border-black/8 hover:bg-black/8"
                    }`}
                  >
                    <FolderOpen className="h-3.5 w-3.5 shrink-0 opacity-70" />
                    <span className="truncate">{selfInfo?.save_dir || "Downloads"}</span>
                  </button>
                </div>
                <div className="flex items-center justify-between pt-1">
                  <div>
                    <div className="text-[12px] font-medium">Auto-Accept Files</div>
                    <div className={`text-[10px] ${subtleText}`}>
                      Skip Accept prompt on trusted Wi-Fi
                    </div>
                  </div>
                  <button
                    onClick={handleToggleAutoAccept}
                    className={`w-10 h-5.5 rounded-full p-0.5 transition-colors cursor-pointer ${
                      selfInfo?.auto_accept
                        ? ""
                        : isDark
                        ? "bg-white/15"
                        : "bg-black/15"
                    }`}
                    style={
                      selfInfo?.auto_accept ? { background: accent.primary } : undefined
                    }
                  >
                    <div
                      className={`h-4.5 w-4.5 rounded-full bg-white shadow transition-transform ${
                        selfInfo?.auto_accept ? "translate-x-4.5" : "translate-x-0"
                      }`}
                    />
                  </button>
                </div>
              </div>
            </div>
          )}
        </div>
      </header>

      {/* Main Content */}
      <main className="relative z-10 flex-1 overflow-y-auto p-6">
        {tab === "nearby" && (
          <div className="h-full flex flex-col items-center justify-between">
            {/* Top helper bar */}
            <div className="w-full flex items-center justify-between">
              <div>
                <h1 className="text-[18px] font-semibold tracking-tight">Nearby Devices</h1>
                <p className={`text-[12px] ${subtleText}`}>
                  Drop files anywhere or click a device on the same Wi-Fi / Hotspot
                </p>
              </div>
              <button
                onClick={pickFilesGeneral}
                className="flex items-center gap-2 px-3.5 py-2 rounded-xl text-[12px] font-semibold text-white shadow-sm transition-transform active:scale-95 cursor-pointer"
                style={{ background: accent.primary }}
              >
                <Upload className="h-3.5 w-3.5" />
                Select Files
              </button>
            </div>

            {/* Radar Field */}
            <div className="relative flex-1 w-full flex items-center justify-center my-4">
              {/* Animated SVG Radar Rings */}
              <svg
                viewBox="0 0 440 440"
                className="pointer-events-none absolute h-[420px] w-[420px]"
                fill="none"
              >
                <circle
                  cx="220"
                  cy="220"
                  r="205"
                  stroke={isDark ? "#ffffff" : "#000000"}
                  strokeOpacity="0.06"
                  strokeWidth="1"
                />
                <circle cx="220" cy="220" r="55" stroke={accent.primary} strokeWidth="1.2">
                  <animate
                    attributeName="r"
                    from="55"
                    to="205"
                    dur="3.6s"
                    begin="0s"
                    repeatCount="indefinite"
                  />
                  <animate
                    attributeName="stroke-opacity"
                    values="0.45;0.18;0"
                    keyTimes="0;0.65;1"
                    dur="3.6s"
                    begin="0s"
                    repeatCount="indefinite"
                  />
                </circle>
                <circle cx="220" cy="220" r="55" stroke={accent.primary} strokeWidth="1.2">
                  <animate
                    attributeName="r"
                    from="55"
                    to="205"
                    dur="3.6s"
                    begin="1.8s"
                    repeatCount="indefinite"
                  />
                  <animate
                    attributeName="stroke-opacity"
                    values="0.45;0.18;0"
                    keyTimes="0;0.65;1"
                    dur="3.6s"
                    begin="1.8s"
                    repeatCount="indefinite"
                  />
                </circle>
              </svg>

              {peers.length === 0 ? (
                <div className="relative z-10 flex flex-col items-center text-center max-w-xs">
                  <div
                    className="h-16 w-16 rounded-2xl flex items-center justify-center mb-3"
                    style={{ background: accent.soft, color: accent.primary }}
                  >
                    <Radio className="h-7 w-7 animate-pulse" strokeWidth={1.6} />
                  </div>
                  <div className="text-[14px] font-medium">Searching nearby...</div>
                  <p className={`text-[12px] mt-1 leading-relaxed ${subtleText}`}>
                    Open LocalFTP on another Mac, Windows, or Linux device on the same Wi-Fi or
                    Hotspot, or use <span className="font-medium">Mobile QR</span> for phones.
                  </p>
                </div>
              ) : (
                <div className="relative z-10 flex flex-wrap items-center justify-center gap-4 max-w-2xl">
                  {peers.map((peer) => (
                    <button
                      key={peer.id}
                      onClick={() => pickFilesForPeer(peer)}
                      className={`group flex flex-col items-center w-36 p-4 rounded-2xl transition-all duration-200 hover:-translate-y-1 cursor-pointer ${glassCard}`}
                    >
                      <div
                        className="h-12 w-12 rounded-2xl flex items-center justify-center mb-2.5 transition-transform group-hover:scale-105"
                        style={{ background: accent.soft, color: accent.primary }}
                      >
                        <OsIcon os={peer.os} className="h-6 w-6" />
                      </div>
                      <span className="text-[13px] font-semibold truncate w-full text-center">
                        {peer.name}
                      </span>
                      <span className={`text-[11px] mt-0.5 ${subtleText}`}>
                        {peer.os}
                      </span>
                      <span
                        className="mt-2 text-[10px] font-medium px-2 py-0.5 rounded-full opacity-0 group-hover:opacity-100 transition-opacity"
                        style={{ background: accent.soft, color: accent.primary }}
                      >
                        Click to send
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </div>

            {/* Self Identity Pill */}
            {selfInfo && (
              <div
                className={`flex items-center gap-3 px-4 py-2 rounded-2xl backdrop-blur-xl ${glassCard}`}
              >
                <div
                  className="h-8 w-8 rounded-xl flex items-center justify-center"
                  style={{ background: accent.soft, color: accent.primary }}
                >
                  <OsIcon os={selfInfo.os} className="h-4 w-4" />
                </div>
                <div className="text-left">
                  <div className="text-[12px] font-semibold">{selfInfo.name}</div>
                  <div className={`text-[11px] ${subtleText}`}>
                    Visible as {selfInfo.os} • {formatNodeCode(selfInfo.id)}
                  </div>
                </div>
              </div>
            )}
          </div>
        )}

        {tab === "mobile" && (
          <div className="h-full grid grid-cols-1 md:grid-cols-2 gap-5 items-stretch">
            {/* Left: QR Code Card */}
            <div
              className={`rounded-2xl p-6 flex flex-col items-center justify-center text-center backdrop-blur-xl ${glassCard}`}
            >
              <div className="bg-white p-3.5 rounded-2xl shadow-sm mb-4">
                {mobileUrl ? (
                  <QRCodeSVG value={mobileUrl} size={168} level="M" />
                ) : (
                  <div className="h-[168px] w-[168px]" />
                )}
              </div>
              <h2 className="text-[15px] font-semibold">Scan from iPhone or Android</h2>
              <p className={`text-[12px] mt-1 max-w-xs ${subtleText}`}>
                Connect your phone to the same Wi-Fi or Hotspot, then scan to send or download
                files directly in your browser.
              </p>
              <div className="mt-3 flex items-center gap-2">
                <div
                  className={`px-3.5 py-1.5 rounded-xl text-[12px] font-medium tracking-wide ${
                    isDark ? "bg-white/8 text-zinc-200" : "bg-black/5 text-zinc-700"
                  }`}
                >
                  {selfInfo ? `Bridge ${formatNodeCode(selfInfo.id)}` : "Starting bridge..."}
                </div>
                {selfInfo && (
                  <button
                    onClick={handleResetMobileToken}
                    title="Reset QR Link"
                    className={`p-1.5 rounded-xl transition-colors cursor-pointer ${
                      isDark ? "bg-white/8 hover:bg-white/14" : "bg-black/5 hover:bg-black/10"
                    }`}
                  >
                    <RefreshCw className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>
            </div>

            {/* Right: Shared & Received Files for Mobile */}
            <div className="flex flex-col gap-4 min-h-0">
              <div className={`flex-1 rounded-2xl p-5 flex flex-col min-h-0 backdrop-blur-xl ${glassCard}`}>
                <div className="flex items-center justify-between mb-3">
                  <div>
                    <h3 className="text-[14px] font-semibold">Shared with Mobile</h3>
                    <p className={`text-[11px] ${subtleText}`}>
                      Drop files here to make them downloadable on your phone
                    </p>
                  </div>
                  <button
                    onClick={pickFilesForMobile}
                    className="px-3 py-1.5 rounded-xl text-[12px] font-medium text-white cursor-pointer"
                    style={{ background: accent.primary }}
                  >
                    Add Files
                  </button>
                </div>

                <div className="flex-1 overflow-y-auto space-y-2 pr-1">
                  {sharedFiles.length === 0 ? (
                    <div
                      className={`h-full min-h-[110px] flex flex-col items-center justify-center rounded-xl border border-dashed ${
                        isDark ? "border-white/12" : "border-black/12"
                      }`}
                    >
                      <FileIcon className={`h-5 w-5 mb-1.5 ${subtleText}`} strokeWidth={1.5} />
                      <span className={`text-[12px] ${subtleText}`}>No files shared yet</span>
                    </div>
                  ) : (
                    sharedFiles.map((f) => {
                      const url = selfInfo
                        ? `http://127.0.0.1:${selfInfo.local_port}/api/shared/${f.id}/preview?t=${encodeURIComponent(selfInfo.mobile_token)}`
                        : null;
                      return (
                        <div
                          key={f.id}
                          className={`flex items-center justify-between px-3 py-2 rounded-xl ${
                            isDark ? "bg-white/[0.05]" : "bg-black/[0.04]"
                          }`}
                        >
                          <div className="flex items-center gap-2.5 min-w-0 pr-3">
                            {renderPreviewThumb(url, f.name, "h-9 w-9")}
                            <div className="min-w-0">
                              <div className="text-[13px] font-medium truncate">{f.name}</div>
                              <div className={`text-[11px] ${subtleText}`}>
                                {formatBytes(f.size)}
                              </div>
                            </div>
                          </div>
                          <button
                            onClick={() =>
                              invoke<SharedFile[]>("remove_shared_file", { fileId: f.id }).then(
                                setSharedFiles
                              )
                            }
                            className="p-1.5 rounded-lg opacity-60 hover:opacity-100 cursor-pointer"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      );
                    })
                  )}
                </div>
              </div>

              <div className={`rounded-2xl p-4 flex flex-col max-h-44 backdrop-blur-xl ${glassCard}`}>
                <div className="flex items-center justify-between mb-2">
                  <h3 className="text-[13px] font-semibold">Received from Mobile</h3>
                  <span className={`text-[11px] ${subtleText}`}>Saved to Downloads</span>
                </div>
                <div className="flex-1 overflow-y-auto space-y-1.5 pr-1">
                  {transfers.filter((t) => t.peer_ip.startsWith("mobile") || t.peer_name === "Mobile Browser").length === 0 ? (
                    <div className={`text-[12px] py-3 text-center ${subtleText}`}>
                      Files sent from your phone will appear here
                    </div>
                  ) : (
                    transfers
                      .filter((t) => t.peer_ip.startsWith("mobile") || t.peer_name === "Mobile Browser")
                      .map((t) => {
                        const url =
                          selfInfo && t.save_path
                            ? `http://127.0.0.1:${selfInfo.local_port}/api/transfers/${t.id}/preview?t=${encodeURIComponent(selfInfo.mobile_token)}`
                            : null;
                        return (
                          <div
                            key={t.id}
                            className={`flex items-center justify-between px-3 py-2 rounded-xl ${
                              isDark ? "bg-white/[0.05]" : "bg-black/[0.04]"
                            }`}
                          >
                            <div className="flex items-center gap-2.5 min-w-0">
                              {renderPreviewThumb(url, t.file_name, "h-8 w-8")}
                              <div className="min-w-0">
                                <div className="text-[12px] font-medium truncate">
                                  {t.file_name}
                                </div>
                                <div className={`text-[10px] ${subtleText}`}>
                                  {formatBytes(t.file_size)}
                                </div>
                              </div>
                            </div>
                            <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 capitalize">
                              {t.status}
                            </span>
                          </div>
                        );
                      })
                  )}
                </div>
              </div>
            </div>
          </div>
        )}

        {tab === "transfers" && (
          <div className="h-full flex flex-col">
            <div className="mb-4">
              <h1 className="text-[18px] font-semibold tracking-tight">Transfers</h1>
              <p className={`text-[12px] ${subtleText}`}>
                Recent incoming and outgoing local transfers
              </p>
            </div>
            <div className="flex-1 overflow-y-auto flex flex-col space-y-2.5 pr-1">
              {transfers.length === 0 ? (
                <div
                  className={`flex-1 min-h-[240px] flex flex-col items-center justify-center rounded-2xl backdrop-blur-xl ${glassCard}`}
                >
                  <Send className={`h-6 w-6 mb-2 ${subtleText}`} strokeWidth={1.5} />
                  <span className={`text-[13px] ${subtleText}`}>No transfers yet</span>
                </div>
              ) : (
                transfers.map((t) => {
                  const pct =
                    t.file_size > 0
                      ? Math.min(100, Math.round((t.bytes_transferred / t.file_size) * 100))
                      : t.status === "completed"
                      ? 100
                      : 0;
                  return (
                    <div
                      key={t.id}
                      className={`rounded-2xl p-4 backdrop-blur-xl ${glassCard}`}
                    >
                      <div className="flex items-center justify-between gap-3">
                        <div className="flex items-center gap-3 min-w-0">
                          {selfInfo && t.save_path && getPreviewKind(t.file_name) ? (
                            renderPreviewThumb(
                              `http://127.0.0.1:${selfInfo.local_port}/api/transfers/${t.id}/preview?t=${encodeURIComponent(selfInfo.mobile_token)}`,
                              t.file_name,
                              "h-9 w-9"
                            )
                          ) : (
                            <div
                              className="h-9 w-9 rounded-xl flex items-center justify-center shrink-0"
                              style={{ background: accent.soft, color: accent.primary }}
                            >
                              {t.direction === "incoming" ? (
                                <ArrowDownLeft className="h-4 w-4" />
                              ) : (
                                <ArrowUpRight className="h-4 w-4" />
                              )}
                            </div>
                          )}
                          <div className="min-w-0">
                            <div className="text-[13px] font-semibold truncate">
                              {t.file_name}
                            </div>
                            <div className={`text-[11px] ${subtleText}`}>
                              {t.direction === "incoming" ? "From" : "To"} {t.peer_name} •{" "}
                              {formatBytes(t.file_size)}
                            </div>
                          </div>
                        </div>
                        <span
                          className={`text-[11px] font-medium px-2.5 py-1 rounded-full capitalize ${
                            t.status === "completed"
                              ? "bg-emerald-500/15 text-emerald-400"
                              : t.status === "failed" || t.status === "rejected"
                              ? "bg-rose-500/15 text-rose-400"
                              : isDark
                              ? "bg-white/10 text-zinc-200"
                              : "bg-black/8 text-zinc-700"
                          }`}
                        >
                          {t.status === "transferring" ? `${pct}%` : t.status}
                        </span>
                      </div>
                      {t.status === "transferring" && (
                        <div className="mt-3 h-1.5 w-full rounded-full bg-black/20 overflow-hidden">
                          <div
                            className="h-full rounded-full transition-all duration-150"
                            style={{ width: `${pct}%`, background: accent.primary }}
                          />
                        </div>
                      )}
                    </div>
                  );
                })
              )}
            </div>
          </div>
        )}
      </main>

      {/* Drag & Drop Full-Window Overlay */}
      {isDragging && (
        <div className="absolute inset-0 z-40 flex items-center justify-center bg-black/45 backdrop-blur-md p-8">
          <div
            className="w-full h-full rounded-3xl border-2 border-dashed flex flex-col items-center justify-center"
            style={{ borderColor: accent.primary, background: accent.soft }}
          >
            <Upload className="h-10 w-10 mb-3" style={{ color: accent.primary }} />
            <div className="text-[17px] font-semibold text-white">Drop files to send</div>
            <p className="text-[12px] text-white/75 mt-1">
              {tab === "mobile"
                ? "Release to share via Mobile QR"
                : "Release to choose a nearby device"}
            </p>
          </div>
        </div>
      )}

      {/* Target Picker Modal after Dropping/Selecting Files */}
      {queuedPaths && (
        <div
          onClick={() => setQueuedPaths(null)}
          className="absolute inset-0 z-40 flex items-center justify-center bg-black/45 backdrop-blur-sm p-6"
        >
          <div
            onClick={(e) => e.stopPropagation()}
            className={`w-full max-w-sm rounded-2xl p-5 backdrop-blur-2xl ${glassCard}`}
          >
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-[15px] font-semibold">
                Send {queuedPaths.length} file{queuedPaths.length > 1 ? "s" : ""}
              </h3>
              <button
                onClick={() => setQueuedPaths(null)}
                className="opacity-60 hover:opacity-100 cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="space-y-2 max-h-60 overflow-y-auto">
              {peers.map((peer) => (
                <button
                  key={peer.id}
                  onClick={() => sendQueuedToPeer(peer.id)}
                  className={`w-full flex items-center justify-between px-3.5 py-2.5 rounded-xl transition-colors cursor-pointer ${
                    isDark ? "bg-white/8 hover:bg-white/14" : "bg-black/5 hover:bg-black/10"
                  }`}
                >
                  <span className="flex items-center gap-2.5">
                    <OsIcon os={peer.os} className="h-4 w-4" />
                    <span className="text-[13px] font-medium">{peer.name}</span>
                  </span>
                  <span className={`text-[11px] ${subtleText}`}>{peer.os}</span>
                </button>
              ))}
              <button
                onClick={shareQueuedToMobile}
                className={`w-full flex items-center justify-between px-3.5 py-2.5 rounded-xl transition-colors cursor-pointer ${
                  isDark ? "bg-white/8 hover:bg-white/14" : "bg-black/5 hover:bg-black/10"
                }`}
              >
                <span className="flex items-center gap-2.5">
                  <QrCode className="h-4 w-4" style={{ color: accent.primary }} />
                  <span className="text-[13px] font-medium">Share via Mobile QR</span>
                </span>
                <span className={`text-[11px] ${subtleText}`}>Phone</span>
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Universal File Preview Modal */}
      {previewTarget && (
        <div
          onClick={() => setPreviewTarget(null)}
          className="fixed inset-0 z-50 flex flex-col items-center justify-center bg-black/80 backdrop-blur-md p-6"
        >
          <div
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-3xl flex items-center justify-between mb-3 text-white gap-3"
          >
            <span className="text-[13px] font-semibold truncate">{previewTarget.name}</span>
            <button
              onClick={() => setPreviewTarget(null)}
              className="px-3 py-1 rounded-full bg-white/15 hover:bg-white/25 text-[12px] font-medium cursor-pointer shrink-0"
            >
              Close
            </button>
          </div>
          <div
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-3xl max-h-[80vh] flex items-center justify-center"
          >
            {previewTarget.kind === "image" && (
              <img
                src={previewTarget.url}
                alt={previewTarget.name}
                className="max-w-full max-h-[78vh] rounded-2xl shadow-2xl object-contain"
              />
            )}
            {previewTarget.kind === "video" && (
              <video
                src={previewTarget.url}
                controls
                autoPlay
                className="max-w-full max-h-[78vh] rounded-2xl shadow-2xl"
              />
            )}
            {previewTarget.kind === "audio" && (
              <div className="w-full max-w-md p-6 rounded-2xl bg-zinc-900/90 border border-white/15 flex flex-col items-center gap-4">
                <Music className="h-10 w-10 text-white/80" />
                <div className="text-[13px] text-white font-medium truncate max-w-full">
                  {previewTarget.name}
                </div>
                <audio src={previewTarget.url} controls autoPlay className="w-full" />
              </div>
            )}
            {previewTarget.kind === "pdf" && (
              <iframe
                src={previewTarget.url}
                title={previewTarget.name}
                className="w-full h-[76vh] rounded-2xl bg-white border-0"
              />
            )}
            {previewTarget.kind === "text" && (
              <pre className="w-full max-h-[76vh] overflow-auto rounded-2xl bg-zinc-900 text-zinc-100 p-4 text-[12px] leading-relaxed border border-white/15 whitespace-pre-wrap break-words">
                {previewText}
              </pre>
            )}
          </div>
        </div>
      )}

      {/* Incoming Transfer Offer Modal */}
      {pendingOffer && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm p-6">
          <div className={`w-full max-w-sm rounded-2xl p-5 backdrop-blur-2xl ${glassCard}`}>
            <div className="flex items-center gap-3 mb-3">
              <div
                className="h-10 w-10 rounded-xl flex items-center justify-center"
                style={{ background: accent.soft, color: accent.primary }}
              >
                <ArrowDownLeft className="h-5 w-5" />
              </div>
              <div>
                <div className="text-[14px] font-semibold">Incoming File</div>
                <div className={`text-[12px] ${subtleText}`}>
                  From {pendingOffer.peer_name}
                </div>
              </div>
            </div>
            <div
              className={`rounded-xl px-3.5 py-2.5 mb-4 ${
                isDark ? "bg-white/[0.06]" : "bg-black/[0.04]"
              }`}
            >
              <div className="text-[13px] font-medium truncate">{pendingOffer.file_name}</div>
              <div className={`text-[11px] ${subtleText}`}>
                {formatBytes(pendingOffer.file_size)}
              </div>
            </div>
            <div className="flex gap-2.5">
              <button
                onClick={() => handleRespondOffer(false)}
                className={`flex-1 py-2 rounded-xl text-[13px] font-medium cursor-pointer ${
                  isDark ? "bg-white/10 hover:bg-white/15" : "bg-black/6 hover:bg-black/10"
                }`}
              >
                Decline
              </button>
              <button
                onClick={() => handleRespondOffer(true)}
                className="flex-1 py-2 rounded-xl text-[13px] font-semibold text-white cursor-pointer"
                style={{ background: accent.primary }}
              >
                Accept
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
