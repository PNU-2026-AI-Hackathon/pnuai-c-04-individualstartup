export async function chooseStlExportPath(defaultFileName: string): Promise<string | null> {
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({
    defaultPath: defaultFileName,
    filters: [{ name: "STL model", extensions: ["stl"] }],
    title: "Export STL"
  });
}

export function stlExportDefaultFileName(sessionTitle?: string): string {
  const baseName = (sessionTitle ?? "")
    .normalize("NFC")
    .replace(/[\u0000-\u001f\u007f/\\:*?"<>|]/g, "-")
    .replace(/[. ]+$/g, "")
    .trim()
    .slice(0, 120);
  return `${baseName || "cadgen-ax-model"}.stl`;
}
