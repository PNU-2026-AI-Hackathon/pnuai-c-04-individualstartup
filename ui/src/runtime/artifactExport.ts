export async function chooseStlExportPath(defaultFileName: string): Promise<string | null> {
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({
    defaultPath: defaultFileName,
    filters: [{ name: "STL model", extensions: ["stl"] }],
    title: "Export STL"
  });
}

export async function chooseGcodeExportPath(defaultFileName: string): Promise<string | null> {
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({
    defaultPath: defaultFileName,
    filters: [{ name: "G-code toolpath", extensions: ["gcode"] }],
    title: "Export G-code"
  });
}

export function stlExportDefaultFileName(sessionTitle?: string): string {
  return exportDefaultBaseName(sessionTitle, "cadgen-ax-model") + ".stl";
}

export function gcodeExportDefaultFileName(sessionTitle?: string): string {
  return exportDefaultBaseName(sessionTitle, "cadgen-ax-toolpath") + ".gcode";
}

function exportDefaultBaseName(sessionTitle: string | undefined, fallback: string): string {
  const baseName = (sessionTitle ?? "")
    .normalize("NFC")
    .replace(/[\u0000-\u001f\u007f/\\:*?"<>|]/g, "-")
    .replace(/[. ]+$/g, "")
    .trim()
    .slice(0, 120);
  return baseName || fallback;
}
