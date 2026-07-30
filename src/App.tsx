import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as api from "./api";
import type { AppConfig, BehaviorConfig, LogLevel, Status } from "./types";

const LOG_LEVELS: LogLevel[] = ["error", "warn", "info", "debug"];

export default function App() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [present, setPresent] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const refreshStatus = useCallback(() => {
    api
      .getStatus()
      .then((s) => {
        setStatus(s);
        // Keep the toggle in sync with the backend's source of truth.
        setEnabled(s.enabled);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    api.getConfig().then(setConfig).catch((e) => setError(String(e)));
    refreshStatus();

    const timer = setInterval(refreshStatus, 1000);
    const unlisten = listen("amow://event", refreshStatus);
    return () => {
      clearInterval(timer);
      unlisten.then((off) => off());
    };
  }, [refreshStatus]);

  const setBehavior = <K extends keyof BehaviorConfig>(
    key: K,
    value: BehaviorConfig[K],
  ) => {
    setConfig((prev) =>
      prev ? { ...prev, behavior: { ...prev.behavior, [key]: value } } : prev,
    );
    setSaved(false);
  };

  const setPresenceGrace = (key: "away_grace_ms" | "return_grace_ms", value: number) => {
    setConfig((prev) =>
      prev ? { ...prev, presence: { ...prev.presence, [key]: value } } : prev,
    );
    setSaved(false);
  };

  const save = async () => {
    if (!config) return;
    setError(null);
    try {
      await api.saveConfig(config);
      setSaved(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const toggleEnabled = async () => {
    const next = !enabled;
    setEnabled(next);
    await api.setEnabled(next);
    refreshStatus();
  };

  const togglePresent = async () => {
    const next = !present;
    setPresent(next);
    await api.setPresent(next);
    refreshStatus();
  };

  if (!config) {
    return (
      <main className="app">
        <p className="muted">{error ? `Error: ${error}` : "Loading…"}</p>
      </main>
    );
  }

  const b = config.behavior;

  return (
    <main className="app">
      <header className="header">
        <h1>Auto-Mute on Walkaway</h1>
        <p className="muted">Privacy-first. Everything runs locally.</p>
      </header>

      <section className="card status">
        <div className={`pill ${status?.protecting ? "pill-on" : ""}`}>
          {status?.protecting ? "Protecting" : status?.enabled ? "Watching" : "Off"}
        </div>
        <dl className="statgrid">
          <div>
            <dt>Presence</dt>
            <dd>{status?.presence ?? "—"}</dd>
          </div>
          <div>
            <dt>Protection</dt>
            <dd>{status?.enabled ? "on" : "off"}</dd>
          </div>
        </dl>
      </section>

      <section className="card">
        <h2>Protection</h2>
        <p className="muted small">
          The master switch. While on, stepping away for the away-grace period
          mutes your mic and disables the camera; returning restores them.
        </p>
        <div className="row">
          <button
            className={enabled ? "btn on" : "btn"}
            onClick={toggleEnabled}
          >
            {enabled ? "Protection on" : "Protection off"}
          </button>
        </div>
      </section>

      <section className="card">
        <h2>Simulate presence</h2>
        <p className="muted small">
          Presence normally comes from the webcam sidecar. This manual toggle is
          a fallback for when it isn't running; it drives the real logic too.
        </p>
        <div className="row">
          <button className={present ? "btn" : "btn on"} onClick={togglePresent}>
            {present ? "At desk" : "Away"}
          </button>
        </div>
      </section>

      <section className="card">
        <h2>Automatic actions</h2>
        <Toggle
          label="Mute mic on walkaway"
          checked={b.auto_mute}
          onChange={(v) => setBehavior("auto_mute", v)}
        />
        <Toggle
          label="Disable camera on walkaway"
          checked={b.auto_camera_off}
          onChange={(v) => setBehavior("auto_camera_off", v)}
        />
        <Toggle
          label="Restore on return"
          checked={b.auto_restore}
          onChange={(v) => setBehavior("auto_restore", v)}
        />
        <Toggle
          label="Notify on action"
          checked={b.notify_on_action}
          onChange={(v) => setBehavior("notify_on_action", v)}
        />
      </section>

      <section className="card">
        <h2>Timing</h2>
        <NumberField
          label="Sample interval (ms)"
          value={b.sample_interval_ms}
          min={50}
          onChange={(v) => setBehavior("sample_interval_ms", v)}
        />
        <NumberField
          label="Away grace (ms)"
          value={config.presence.away_grace_ms}
          min={100}
          onChange={(v) => setPresenceGrace("away_grace_ms", v)}
        />
        <NumberField
          label="Return grace (ms)"
          value={config.presence.return_grace_ms}
          min={100}
          onChange={(v) => setPresenceGrace("return_grace_ms", v)}
        />
      </section>

      <section className="card">
        <h2>Logging</h2>
        <label className="field">
          <span>Level</span>
          <select
            value={config.logging.level}
            onChange={(e) =>
              setConfig({
                ...config,
                logging: { level: e.target.value as LogLevel },
              })
            }
          >
            {LOG_LEVELS.map((lvl) => (
              <option key={lvl} value={lvl}>
                {lvl}
              </option>
            ))}
          </select>
        </label>
      </section>

      {error && <p className="error">{error}</p>}

      <footer className="footer">
        <button className="btn primary" onClick={save}>
          Save settings
        </button>
        {saved && <span className="saved">Saved ✓</span>}
      </footer>
    </main>
  );
}

function Toggle(props: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="field">
      <span>{props.label}</span>
      <input
        type="checkbox"
        checked={props.checked}
        onChange={(e) => props.onChange(e.target.checked)}
      />
    </label>
  );
}

function NumberField(props: {
  label: string;
  value: number;
  min: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="field">
      <span>{props.label}</span>
      <input
        type="number"
        min={props.min}
        value={props.value}
        onChange={(e) => {
          const parsed = Number(e.target.value);
          if (Number.isFinite(parsed)) props.onChange(parsed);
        }}
      />
    </label>
  );
}
