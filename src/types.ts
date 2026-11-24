export interface PeerInfo {
  id: string;
  name: string;
  os: string;
  ip: string;
  port: number;
  https_enabled: boolean;
  last_seen: number;
}

export interface SelfInfo {
  id: string;
  name: string;
  os: string;
  ip: string;
  port: number;
  local_port: number;
  save_dir: string;
  mobile_token: string;
  auto_accept: boolean;
  https_enabled: boolean;
}

export type TransferDirection = "incoming" | "outgoing";
export type TransferStatus =
  | "pending"
  | "transferring"
  | "completed"
  | "rejected"
  | "failed";

export interface TransferItem {
  id: string;
  file_name: string;
  file_size: number;
  bytes_transferred: number;
  peer_name: string;
  peer_ip: string;
  direction: TransferDirection;
  status: TransferStatus;
  save_path?: string | null;
  error?: string | null;
}

export interface SharedFile {
  id: string;
  name: string;
  size: number;
  path: string;
}

export type SurfaceMode = "dark" | "light";

export interface AccentTheme {
  id: string;
  label: string;
  primary: string;
  soft: string;
  ring: string;
  swatch: string;
}
