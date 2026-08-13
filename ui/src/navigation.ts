export type WorkspaceView = "workspace" | "sessions" | "artifacts" | "logs" | "settings";

export function toHistoryPath(uiUrl: string, baseUrl: string): string {
  const url = new URL(uiUrl, baseUrl);
  return `${url.pathname}${url.search}`;
}

export function sessionIdFromUrl(url: string, baseUrl: string): string | null {
  const parsed = new URL(url, baseUrl);
  const match = parsed.pathname.match(/^\/sessions\/([^/]+)/);
  return match?.[1] ?? null;
}

export function workspaceViewFromUrl(url: string, baseUrl: string): WorkspaceView {
  const parsed = new URL(url, baseUrl);
  const view = parsed.searchParams.get("view");
  return isWorkspaceView(view) ? view : "workspace";
}

export function sessionPathWithView(sessionId: string, view: WorkspaceView): string {
  const path = `/sessions/${sessionId}`;
  return view === "workspace" ? path : `${path}?view=${view}`;
}

function isWorkspaceView(value: string | null): value is WorkspaceView {
  return value === "workspace" || value === "sessions" || value === "artifacts" || value === "logs" || value === "settings";
}
