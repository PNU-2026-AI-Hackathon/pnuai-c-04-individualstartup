import { Box, Home, List, Plus, ScrollText } from "lucide-react";
import type { WorkspaceView } from "../navigation";

export function WorkspaceNav({
  busy,
  onCreateSession,
  view,
  onNavigate
}: {
  busy: boolean;
  onCreateSession: () => void;
  view: WorkspaceView;
  onNavigate: (view: WorkspaceView) => void;
}) {
  const items: Array<{ view: WorkspaceView; label: string; icon: typeof Home }> = [
    { view: "workspace", label: "Workspace", icon: Home },
    { view: "sessions", label: "Sessions", icon: List },
    { view: "artifacts", label: "Artifacts", icon: Box },
    { view: "logs", label: "Logs", icon: ScrollText }
  ];
  return (
    <nav className="workspace-nav" aria-label="Workspace navigation">
      {items.map((item) => {
        const Icon = item.icon;
        return (
          <button
            className={view === item.view ? "active" : ""}
            key={item.view}
            onClick={() => onNavigate(item.view)}
            title={item.label}
          >
            <Icon size={16} /> {item.label}
          </button>
        );
      })}
      <button
        className="workspace-nav-action"
        onClick={onCreateSession}
        disabled={busy}
        title="Create session"
      >
        <Plus size={16} /> New session
      </button>
    </nav>
  );
}
