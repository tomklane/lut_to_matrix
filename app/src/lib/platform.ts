// Native dialogs, clipboard and file drops, with plain-browser fallbacks for development.

import { inTauri } from "./api";

export interface FileFilter {
  name: string;
  extensions: string[];
}

export const IMAGE_FILTER: FileFilter = {
  name: "Still",
  extensions: ["tif", "tiff", "exr", "png", "jpg", "jpeg", "dpx", "TIF", "TIFF", "EXR", "PNG", "JPG", "JPEG", "DPX"],
};
export const IMAGE_RE = /\.(tiff?|exr|png|jpe?g|dpx)$/i;
export const DCTL_FILTER: FileFilter = { name: "DCTL", extensions: ["dctl"] };

export const LUT_FILTER: FileFilter = { name: "LUT", extensions: ["cube", "3dl", "dat", "CUBE", "3DL", "DAT"] };

export async function openFile(filters: FileFilter[], defaultPath?: string): Promise<string | null> {
  if (!inTauri) return devPrompt("Path to open");
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ multiple: false, directory: false, filters, defaultPath });
  return typeof r === "string" ? r : null;
}

export async function openFiles(filters: FileFilter[]): Promise<string[]> {
  if (!inTauri) {
    const p = devPrompt("Paths to open");
    return p ? [p] : [];
  }
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ multiple: true, directory: false, filters });
  return Array.isArray(r) ? r : r ? [r] : [];
}

export async function saveFile(defaultPath: string, filters: FileFilter[]): Promise<string | null> {
  if (!inTauri) return devPrompt("Save as", defaultPath);
  const { save } = await import("@tauri-apps/plugin-dialog");
  return (await save({ defaultPath, filters })) ?? null;
}

export async function confirmAsk(message: string, title: string, okLabel: string): Promise<boolean> {
  if (!inTauri) return window.confirm(message);
  const { ask } = await import("@tauri-apps/plugin-dialog");
  return ask(message, { title, kind: "warning", okLabel, cancelLabel: "Cancel" });
}

export async function copyText(text: string): Promise<void> {
  if (inTauri) {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    return writeText(text);
  }
  await navigator.clipboard?.writeText(text);
}

/** Calls `onDrop` with the paths dropped on the window; `onHover(true/false)` while dragging. */
export async function listenForDrops(onDrop: (paths: string[]) => void, onHover: (over: boolean) => void) {
  if (!inTauri) {
    window.addEventListener("dragover", (e) => {
      e.preventDefault();
      onHover(true);
    });
    window.addEventListener("dragleave", () => onHover(false));
    window.addEventListener("drop", (e) => {
      e.preventDefault();
      onHover(false);
      const f = e.dataTransfer?.files?.[0];
      if (f) onDrop([f.name]);
    });
    return;
  }
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  await getCurrentWebview().onDragDropEvent((event) => {
    const p = event.payload;
    if (p.type === "enter" || p.type === "over") onHover(true);
    else if (p.type === "leave") onHover(false);
    else if (p.type === "drop") {
      onHover(false);
      onDrop(p.paths);
    }
  });
}

function devPrompt(label: string, value = ""): string | null {
  return window.prompt(label, value);
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function dirName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : "";
}

/** Join a folder and a file name with the folder's own separator (Windows or POSIX). */
export function joinPath(dir: string, name: string): string {
  if (!dir) return name;
  const sep = dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
}
