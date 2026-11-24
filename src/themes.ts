import type { AccentTheme } from "./types";

export const ACCENT_THEMES: AccentTheme[] = [
  {
    id: "ocean",
    label: "Ocean Blue",
    primary: "#3b82f6",
    soft: "rgba(59, 130, 246, 0.16)",
    ring: "rgba(59, 130, 246, 0.35)",
    swatch: "#3b82f6",
  },
  {
    id: "emerald",
    label: "Emerald Mint",
    primary: "#10b981",
    soft: "rgba(16, 185, 129, 0.16)",
    ring: "rgba(16, 185, 129, 0.35)",
    swatch: "#10b981",
  },
  {
    id: "violet",
    label: "Violet Dusk",
    primary: "#8b5cf6",
    soft: "rgba(139, 92, 246, 0.16)",
    ring: "rgba(139, 92, 246, 0.35)",
    swatch: "#8b5cf6",
  },
  {
    id: "rose",
    label: "Rose Quartz",
    primary: "#f43f5e",
    soft: "rgba(244, 63, 94, 0.16)",
    ring: "rgba(244, 63, 94, 0.35)",
    swatch: "#f43f5e",
  },
  {
    id: "amber",
    label: "Amber Gold",
    primary: "#f59e0b",
    soft: "rgba(245, 158, 11, 0.16)",
    ring: "rgba(245, 158, 11, 0.35)",
    swatch: "#f59e0b",
  },
];

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
