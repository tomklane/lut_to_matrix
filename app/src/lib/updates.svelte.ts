// In-app updates (Tauri updater, DECISIONS D41): check the latest published GitHub
// release, download it, verify its signature, install it and relaunch.

import { inTauri } from "./api";

type Update = import("@tauri-apps/plugin-updater").Update;

class Updates {
  /** Check once at launch (remembered setting). */
  auto = $state(true);
  checking = $state(false);
  /** A newer version is available. */
  available = $state<{ version: string; notes: string; date: string | null } | null>(null);
  /** Download progress 0–1 while installing; null otherwise. */
  progress = $state<number | null>(null);
  /** Last result of a check the user asked for ("up to date", an error). */
  message = $state<string | null>(null);
  /** The banner was dismissed for this version. */
  dismissed = $state<string | null>(null);

  private update: Update | null = null;

  /** `manual`: the user asked, so say "up to date" / show errors; a launch check stays quiet. */
  async check(manual = false) {
    if (!inTauri || this.checking) return;
    this.checking = true;
    if (manual) this.message = null;
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const u = await check();
      this.update = u;
      this.available = u ? { version: u.version, notes: u.body ?? "", date: u.date ?? null } : null;
      if (manual) {
        this.dismissed = null;
        if (!u) this.message = "LUT Matrix is up to date.";
      }
    } catch (e) {
      if (manual) this.message = `Couldn't check for updates: ${e}`;
    } finally {
      this.checking = false;
    }
  }

  /** Download, verify, install, relaunch. */
  async install() {
    if (!this.update || this.progress !== null) return;
    this.progress = 0;
    this.message = null;
    let total = 0;
    let done = 0;
    try {
      await this.update.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") {
          done += ev.data.chunkLength;
          this.progress = total > 0 ? Math.min(1, done / total) : 0;
        } else if (ev.event === "Finished") this.progress = 1;
      });
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch (e) {
      this.message = `The update didn't install: ${e}`;
      this.progress = null;
    }
  }
}

export const updates = new Updates();

// Development only (`npm run dev`, ?update): show the banner with made-up notes.
if (import.meta.env.DEV && typeof location !== "undefined" && new URLSearchParams(location.search).has("update")) {
  updates.available = { version: "9.9.9", notes: "## Added\n- **Something new**: what it does.\n- Another `change`.", date: null };
}
