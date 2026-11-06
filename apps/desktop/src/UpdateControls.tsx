import { useEffect, useRef, useState } from "react";
import { Download, LoaderCircle, RefreshCw } from "lucide-react";
import { errorMessage } from "./bridge";
import type { DesktopApi, UpdateStatus } from "./bridge";
import { bytes } from "./format";
import { version } from "../package.json";

type UpdateApi = Pick<DesktopApi, "updateStatus" | "subscribeUpdates" | "checkUpdate" | "downloadUpdate" | "installUpdate">;
export function UpdateControls({ api }: { api: UpdateApi }) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const alive = useRef(false);
  const restart = useRef<HTMLButtonElement>(null);
  const install = useRef<HTMLButtonElement>(null);
  const wasConfirming = useRef(false);
  function accept(next: UpdateStatus) {
    if (alive.current) setStatus((previous) => !previous || next.revision >= previous.revision ? next : previous);
  }
  useEffect(() => {
    alive.current = true;
    let active = true;
    let unsubscribe: (() => void) | undefined;
    void (async () => {
      try {
        const stop = await api.subscribeUpdates((next) => {
          if (active) setStatus((previous) => !previous || next.revision >= previous.revision ? next : previous);
        });
        if (!active) { stop(); return; }
        unsubscribe = stop;
        const next = await api.updateStatus();
        if (active) setStatus((previous) => !previous || next.revision >= previous.revision ? next : previous);
      } catch (reason) {
        if (active) setError(errorMessage(reason));
      }
    })();
    return () => { active = false; alive.current = false; unsubscribe?.(); };
  }, [api]);
  useEffect(() => {
    if (confirm) restart.current?.focus();
    else if (wasConfirming.current) install.current?.focus();
    wasConfirming.current = confirm;
  }, [confirm]);
  const phase = status?.phase;
  const busy = working || phase === "checking" || phase === "downloading" || phase === "installing";
  const canInstall = phase === "ready" || (phase === "failed" && status?.retry === "install");
  const canDownload = phase === "available" || (phase === "failed" && status?.retry === "download");
  async function run(action: "check" | "download" | "install") {
    if (busy) return;
    setWorking(true);
    setError(null);
    setConfirm(false);
    try {
      if (action === "install") await api.installUpdate();
      else accept(await (action === "check" ? api.checkUpdate() : api.downloadUpdate()));
    } catch (reason) {
      if (alive.current) {
        setError(errorMessage(reason));
        try { accept(await api.updateStatus()); } catch { /* Keep the actionable operation error. */ }
      }
    } finally {
      if (alive.current) setWorking(false);
    }
  }
  const message = phase === "current" ? "You’re up to date."
    : phase === "checking" ? "Checking for updates…"
    : phase === "available" ? `RusTor ${status?.version} is available.`
    : phase === "ready" ? `RusTor ${status?.version} is verified and ready to install.`
    : phase === "installing" ? "Saving your session and installing the update…"
    : phase === "downloading" ? "Downloading update…"
    : phase === "idle" ? "Check for a newer version of RusTor."
    : null;
  return (
    <section className="update-controls" aria-labelledby="update-title" aria-busy={busy}>
      <div className="update-heading">
        <h3 id="update-title">Updates</h3>
        <span>RusTor {version}</span>
      </div>
      <p className="settings-help" role="status">{message}</p>
      {(error || status?.error) && (
        <p className="settings-help" role={phase === "unavailable" ? "status" : "alert"}>
          {error ?? status?.error?.message}
        </p>
      )}
      {status?.notes && (
        <details className="update-notes"><summary>Release notes</summary><p>{status.notes}</p></details>
      )}
      {phase === "downloading" && status && (
        <div className="update-progress">
          <progress aria-label="Update download" max={status.total ?? undefined}
            value={status.total ? Math.min(status.downloaded, status.total) : undefined} />
          <span>{bytes(status.downloaded)}{status.total ? ` of ${bytes(status.total)}` : " downloaded"}</span>
          <p className="settings-help">The package will be verified before installation becomes available.</p>
        </div>
      )}
      {confirm ? (
        <div className="update-confirm" role="group" aria-label="Restart to install update">
          <p>Active transfers will stop while RusTor updates. Your session and files will be kept; downloads will be rechecked when the app reopens. Save any settings changes before restarting.</p>
          <div className="update-actions">
            <button type="button" className="button secondary" onClick={() => { setConfirm(false); install.current?.focus(); }}>Keep using RusTor</button>
            <button ref={restart} type="button" className="button primary" onClick={() => void run("install")}>Restart now</button>
          </div>
        </div>
      ) : (
        <div className="update-actions">
          {status && (phase === "idle" || phase === "current" || phase === "checking" || (phase === "failed" && status.retry === "check")) && !status.transfers_stopped && (
            <button type="button" className="button secondary" disabled={busy} onClick={() => void run("check")}>
              {busy ? <LoaderCircle className="spin" size={16} aria-hidden="true" /> : <RefreshCw size={16} aria-hidden="true" />}
              {phase === "checking" ? "Checking…" : "Check for updates"}
            </button>
          )}
          {canDownload && <button type="button" className="button secondary" disabled={busy} onClick={() => void run("download")}><Download size={16} aria-hidden="true" />{phase === "failed" ? "Retry download" : "Download update"}</button>}
          {canInstall && <button ref={install} type="button" className="button primary" disabled={busy} onClick={() => setConfirm(true)}>{phase === "failed" ? "Retry installation" : "Install and restart"}</button>}
        </div>
      )}
    </section>
  );
}
