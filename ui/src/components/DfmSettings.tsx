import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, Download, FileInput, FolderOpen, RotateCcw, Save, Search, Upload, XCircle } from "lucide-react";
import type {
  DfmProfileDocument,
  DfmProfileValidation,
  DfmSettingsBackendClient,
  PrusaSlicerValidation
} from "../backendClient";
import {
  DFM_PROFILE_CATEGORIES,
  macAppExecutablePath,
  parseDfmProfile,
  updateDfmProfileValue,
  type DfmProfileEntry
} from "../dfmProfile";

type EditorMode = "guided" | "advanced";

export interface DfmSettingsDialog {
  chooseExecutable(): Promise<string | null>;
  chooseProfileToImport(): Promise<string | null>;
  chooseProfileExportPath(): Promise<string | null>;
}

export function DfmSettings({
  backend,
  dialog = tauriDfmSettingsDialog
}: {
  backend: DfmSettingsBackendClient;
  dialog?: DfmSettingsDialog;
}) {
  const [loading, setLoading] = useState(true);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [executablePath, setExecutablePath] = useState("");
  const [savedExecutablePath, setSavedExecutablePath] = useState("");
  const [executableValidation, setExecutableValidation] = useState<PrusaSlicerValidation | null>(null);
  const [bundleSelection, setBundleSelection] = useState<string | null>(null);
  const [profileContents, setProfileContents] = useState("");
  const [savedProfileContents, setSavedProfileContents] = useState("");
  const [profileValidation, setProfileValidation] = useState<DfmProfileValidation | null>(null);
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState<(typeof DFM_PROFILE_CATEGORIES)[number] | "All">("All");
  const [editorMode, setEditorMode] = useState<EditorMode>("guided");

  const parsedProfile = useMemo(() => parseDfmProfile(profileContents), [profileContents]);
  const visibleEntries = useMemo(() => {
    const query = search.trim().toLowerCase();
    return parsedProfile.entries.filter((entry) => {
      if (category !== "All" && entry.category !== category) return false;
      return !query || entry.key.toLowerCase().includes(query) || entry.value.toLowerCase().includes(query);
    });
  }, [category, parsedProfile.entries, search]);
  const executableDirty = executablePath !== savedExecutablePath;
  const profileDirty = profileContents !== savedProfileContents;
  const appConversion = macAppExecutablePath(executablePath);

  useEffect(() => {
    let cancelled = false;
    backend.getDfmSettings().then((settings) => {
      if (cancelled) return;
      const path = settings.prusaslicerExecutable ?? "";
      setExecutablePath(path);
      setSavedExecutablePath(path);
      setExecutableValidation(settings.executableValidation);
      applyProfile(settings.profile);
      setLoading(false);
    }).catch((caught) => {
      if (!cancelled) {
        setError(errorMessage(caught));
        setLoading(false);
      }
    });
    return () => { cancelled = true; };
  }, [backend]);

  function applyProfile(profile: DfmProfileDocument) {
    setProfileContents(profile.contents);
    setSavedProfileContents(profile.contents);
    setProfileValidation({ hash: profile.hash, keySettings: profile.keySettings });
  }

  function editExecutablePath(value: string) {
    setExecutablePath(value);
    setExecutableValidation(null);
    setBundleSelection(null);
    setNotice(null);
  }

  async function runAction(name: string, work: () => Promise<void>) {
    setBusyAction(name);
    setError(null);
    setNotice(null);
    try {
      await work();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusyAction(null);
    }
  }

  async function chooseExecutable() {
    await runAction("pick-executable", async () => {
      const selectedPath = await dialog.chooseExecutable();
      if (selectedPath === null) return;
      const resolvedPath = macAppExecutablePath(selectedPath) ?? selectedPath;
      setExecutablePath(resolvedPath);
      setExecutableValidation(null);
      setBundleSelection(resolvedPath !== selectedPath ? selectedPath : null);
    });
  }

  async function validateExecutable() {
    await runAction("validate-executable", async () => {
      const path = executableForRequest(executablePath);
      const result = await backend.validatePrusaSlicerExecutable({ path });
      setExecutablePath(result.path);
      setExecutableValidation(result);
      setNotice(`PrusaSlicer ${result.version} is executable and compatible.`);
    });
  }

  async function saveExecutable() {
    await runAction("save-executable", async () => {
      const path = executableForRequest(executablePath);
      const result = await backend.savePrusaSlicerExecutable({ path });
      setExecutablePath(result.path);
      setSavedExecutablePath(result.path);
      setExecutableValidation(result);
      setNotice("PrusaSlicer executable saved.");
    });
  }

  function editProfile(nextContents: string) {
    setProfileContents(nextContents);
    setProfileValidation(null);
    setNotice(null);
  }

  async function validateProfile() {
    await runAction("validate-profile", async () => {
      requireValidLocalProfile(profileContents);
      const result = await backend.validateDfmProfile({ contents: profileContents });
      setProfileValidation(result);
      setNotice("Profile syntax and required settings are valid.");
    });
  }

  async function saveProfile() {
    await runAction("save-profile", async () => {
      requireValidLocalProfile(profileContents);
      const result = await backend.saveDfmProfile({ contents: profileContents });
      applyProfile(result);
      setNotice("DFM profile saved.");
    });
  }

  async function importProfile() {
    await runAction("import-profile", async () => {
      const path = await dialog.chooseProfileToImport();
      if (path === null) return;
      const result = await backend.importDfmProfile({ path });
      requireValidLocalProfile(result.contents);
      setProfileContents(result.contents);
      setProfileValidation(null);
      setNotice(`Imported ${result.sourcePath}. Review and save to apply it.`);
    });
  }

  async function exportProfile() {
    await runAction("export-profile", async () => {
      requireValidLocalProfile(profileContents);
      const path = await dialog.chooseProfileExportPath();
      if (path === null) return;
      const result = await backend.exportDfmProfile({ path, contents: profileContents });
      setNotice(`Exported profile to ${result.path}.`);
    });
  }

  async function restoreDefaultProfile() {
    await runAction("restore-default", async () => {
      const result = await backend.restoreDefaultDfmProfile();
      applyProfile(result);
      setNotice("Default DFM profile restored and saved.");
    });
  }

  if (loading) {
    return <section className="settings-view"><p>Loading DFM settings…</p></section>;
  }

  return (
    <section className="settings-view" aria-labelledby="dfm-settings-title">
      <header className="settings-heading">
        <div>
          <h2 id="dfm-settings-title">DFM validation</h2>
          <p>Configure the exact PrusaSlicer binary and profile used during finalization.</p>
        </div>
      </header>

      {error ? <div className="error" role="alert">{error}</div> : null}
      {notice ? <div className="settings-notice" role="status">{notice}</div> : null}

      <section className="settings-card" aria-labelledby="prusaslicer-heading">
        <header>
          <div>
            <h3 id="prusaslicer-heading">PrusaSlicer executable</h3>
            <p>An absolute path is required. PATH aliases are never used.</p>
          </div>
          <ValidationBadge validation={executableValidation} dirty={executableDirty} />
        </header>
        <label className="settings-field">
          <span>Absolute executable path</span>
          <div className="path-input-row">
            <input
              aria-label="PrusaSlicer executable path"
              autoComplete="off"
              onChange={(event) => editExecutablePath(event.target.value)}
              placeholder="/Applications/PrusaSlicer.app/Contents/MacOS/PrusaSlicer"
              spellCheck={false}
              value={executablePath}
            />
            <button disabled={Boolean(busyAction)} onClick={chooseExecutable} title="Choose PrusaSlicer executable">
              <FolderOpen size={16} /> Choose…
            </button>
          </div>
        </label>
        {bundleSelection ? (
          <p className="settings-help success">Selected app bundle <code>{bundleSelection}</code>; using its internal CLI.</p>
        ) : null}
        {appConversion ? (
          <div className="settings-help warning-inline">
            <span>A macOS .app is a bundle, not an executable. The internal CLI will be used:</span>
            <code>{appConversion}</code>
            <button onClick={() => editExecutablePath(appConversion)}>Use internal CLI</button>
          </div>
        ) : null}
        <div className="settings-actions">
          <button disabled={Boolean(busyAction) || !executablePath.trim()} onClick={validateExecutable}>
            <CheckCircle2 size={16} /> Validate
          </button>
          <button className="primary" disabled={Boolean(busyAction) || !executableDirty || !executablePath.trim()} onClick={saveExecutable}>
            <Save size={16} /> Save executable
          </button>
        </div>
        {executableValidation ? (
          <dl className="settings-summary">
            <div><dt>Status</dt><dd>Ready</dd></div>
            <div><dt>Version</dt><dd>{executableValidation.version}</dd></div>
            <div><dt>Validated path</dt><dd><code>{executableValidation.path}</code></dd></div>
          </dl>
        ) : null}
      </section>

      <section className="settings-card profile-card" aria-labelledby="profile-heading">
        <header>
          <div>
            <h3 id="profile-heading">Slicing profile</h3>
            <p>Search common settings or edit every key/value in the advanced editor.</p>
          </div>
          <span className={profileDirty ? "dirty-badge" : "saved-badge"}>{profileDirty ? "Unsaved" : "Saved"}</span>
        </header>

        <div className="profile-summary" aria-label="Active DFM profile summary">
          <div><span>Profile hash</span><code>{profileValidation?.hash ?? "Not validated"}</code></div>
          {Object.entries(profileValidation?.keySettings ?? {}).map(([key, value]) => (
            <div key={key}><span>{humanizeKey(key)}</span><strong>{value}</strong></div>
          ))}
        </div>

        <div className="profile-toolbar">
          <div className="segmented-control" aria-label="Profile editor mode">
            <button className={editorMode === "guided" ? "active" : ""} onClick={() => setEditorMode("guided")}>Guided</button>
            <button className={editorMode === "advanced" ? "active" : ""} onClick={() => setEditorMode("advanced")}>Advanced key/value</button>
          </div>
          <div className="profile-file-actions">
            <button disabled={Boolean(busyAction)} onClick={importProfile}><FileInput size={15} /> Import INI</button>
            <button disabled={Boolean(busyAction)} onClick={exportProfile}><Download size={15} /> Export INI</button>
            <button disabled={Boolean(busyAction)} onClick={restoreDefaultProfile}><RotateCcw size={15} /> Restore defaults</button>
          </div>
        </div>

        {editorMode === "guided" ? (
          <>
            <div className="profile-filters">
              <label className="search-field">
                <Search size={15} />
                <input aria-label="Search profile settings" onChange={(event) => setSearch(event.target.value)} placeholder="Search key or value" value={search} />
              </label>
              <label>
                <span className="sr-only">Profile category</span>
                <select aria-label="Profile category" onChange={(event) => setCategory(event.target.value as typeof category)} value={category}>
                  <option value="All">All categories</option>
                  {DFM_PROFILE_CATEGORIES.map((value) => <option key={value} value={value}>{value}</option>)}
                </select>
              </label>
            </div>
            <div className="profile-entry-list">
              {visibleEntries.map((entry) => (
                <ProfileEntryInput
                  entry={entry}
                  key={entry.key}
                  onChange={(value) => editProfile(updateDfmProfileValue(profileContents, entry.key, value))}
                />
              ))}
              {visibleEntries.length === 0 ? <p className="empty-settings">No profile settings match this filter.</p> : null}
            </div>
          </>
        ) : (
          <label className="advanced-profile-editor">
            <span>Complete profile.ini</span>
            <textarea
              aria-label="Complete profile.ini"
              onChange={(event) => editProfile(event.target.value)}
              spellCheck={false}
              value={profileContents}
            />
          </label>
        )}

        {parsedProfile.errors.length > 0 ? (
          <div className="profile-errors" role="alert">
            <strong>Profile cannot be saved</strong>
            <ul>{parsedProfile.errors.map((message) => <li key={message}>{message}</li>)}</ul>
          </div>
        ) : null}
        <div className="settings-actions">
          <button disabled={Boolean(busyAction) || parsedProfile.errors.length > 0} onClick={validateProfile}>
            <Upload size={16} /> Validate profile
          </button>
          <button className="primary" disabled={Boolean(busyAction) || !profileDirty || parsedProfile.errors.length > 0} onClick={saveProfile}>
            <Save size={16} /> Save profile
          </button>
        </div>
      </section>
    </section>
  );
}

function ProfileEntryInput({ entry, onChange }: { entry: DfmProfileEntry; onChange: (value: string) => void }) {
  let input;
  if (entry.valueType === "boolean") {
    input = (
      <select aria-label={entry.key} onChange={(event) => onChange(event.target.value)} value={entry.value}>
        <option value="1">True</option>
        <option value="0">False</option>
      </select>
    );
  } else if (entry.valueType === "enum") {
    input = (
      <select aria-label={entry.key} onChange={(event) => onChange(event.target.value)} value={entry.value}>
        {!entry.options?.includes(entry.value) ? <option value={entry.value}>{entry.value}</option> : null}
        {entry.options?.map((option) => <option key={option} value={option}>{option}</option>)}
      </select>
    );
  } else if (entry.valueType === "percent") {
    input = <PercentInput entry={entry} onChange={onChange} />;
  } else if (entry.valueType === "multi") {
    input = (
      <div className="multi-value-input">
        <textarea aria-label={entry.key} onChange={(event) => onChange(event.target.value)} rows={2} value={entry.value} />
        <small>Comma-separated values</small>
      </div>
    );
  } else {
    input = (
      <input
        aria-label={entry.key}
        inputMode={entry.valueType === "number" ? "decimal" : undefined}
        onChange={(event) => onChange(event.target.value)}
        type={entry.valueType === "number" ? "number" : "text"}
        value={entry.value}
      />
    );
  }
  return (
    <label className="profile-entry">
      <span><strong>{humanizeKey(entry.key)}</strong><code>{entry.key}</code></span>
      <small>{entry.category} · {entry.valueType === "multi" ? "multiple values" : entry.valueType}</small>
      {input}
    </label>
  );
}

function PercentInput({ entry, onChange }: { entry: DfmProfileEntry; onChange: (value: string) => void }) {
  return (
    <div className="unit-input">
      <input aria-label={entry.key} inputMode="decimal" onChange={(event) => onChange(`${event.target.value}%`)} type="number" value={entry.value.slice(0, -1)} />
      <span>%</span>
    </div>
  );
}

function ValidationBadge({ validation, dirty }: { validation: PrusaSlicerValidation | null; dirty: boolean }) {
  if (validation && !dirty) return <span className="validation-badge valid"><CheckCircle2 size={14} /> Validated</span>;
  return <span className="validation-badge invalid"><XCircle size={14} /> {dirty ? "Needs validation" : "Not configured"}</span>;
}

function executableForRequest(path: string): string {
  const trimmed = path.trim();
  if (!trimmed) throw new Error("PrusaSlicer executable path is required.");
  const resolved = macAppExecutablePath(trimmed) ?? trimmed;
  if (!resolved.startsWith("/")) throw new Error("PrusaSlicer executable must be an absolute path.");
  return resolved;
}

function requireValidLocalProfile(contents: string) {
  const result = parseDfmProfile(contents);
  if (result.errors.length > 0) throw new Error(result.errors.join(" "));
}

function humanizeKey(key: string): string {
  return key.replace(/_/g, " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

const tauriDfmSettingsDialog: DfmSettingsDialog = {
  async chooseExecutable() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    return open({ directory: false, multiple: false, title: "Choose PrusaSlicer executable" });
  },
  async chooseProfileToImport() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    return open({
      directory: false,
      filters: [{ name: "PrusaSlicer profile", extensions: ["ini"] }],
      multiple: false,
      title: "Import PrusaSlicer profile"
    });
  },
  async chooseProfileExportPath() {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({
      defaultPath: "profile.ini",
      filters: [{ name: "PrusaSlicer profile", extensions: ["ini"] }],
      title: "Export PrusaSlicer profile"
    });
  }
};
