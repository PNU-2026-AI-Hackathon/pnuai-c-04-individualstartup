import { useState } from "react";
import {
  Archive,
  ArchiveRestore,
  Box,
  Check,
  Clock,
  Copy,
  Edit3,
  FileText,
  MoreVertical,
  Plus,
  Search,
  Trash2,
  X
} from "lucide-react";
import type { CadSessionListItem } from "../protocol";
import type { WorkspaceView } from "../navigation";

export function SessionRail({
  sessions,
  activeSessionId,
  query,
  showArchived,
  busy,
  open,
  view,
  onQueryChange,
  onShowArchivedChange,
  onCreateSession,
  onOpenSession,
  onArchiveChange,
  onRename,
  onDuplicate,
  onDelete,
  onNavigate,
  onClose
}: {
  sessions: CadSessionListItem[];
  activeSessionId: string;
  query: string;
  showArchived: boolean;
  busy: boolean;
  open: boolean;
  view: WorkspaceView;
  onQueryChange: (query: string) => void;
  onShowArchivedChange: (showArchived: boolean) => void;
  onCreateSession: () => void;
  onOpenSession: (sessionId: string) => void;
  onArchiveChange: (sessionId: string, archived: boolean) => void | Promise<void>;
  onRename: (sessionId: string, title: string) => void | Promise<void>;
  onDuplicate: (sessionId: string) => void | Promise<void>;
  onDelete: (sessionId: string) => void | Promise<void>;
  onNavigate: (view: WorkspaceView) => void;
  onClose: () => void;
}) {
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [openMenuSessionId, setOpenMenuSessionId] = useState<string | null>(null);
  const visibleSessions = sessions
    .filter((session) => showArchived || !session.archived || session.id === activeSessionId)
    .slice()
    .sort((left, right) => sessionSortStamp(right).localeCompare(sessionSortStamp(left)));
  const navItems: Array<{ view: WorkspaceView; label: string; icon: typeof Box }> = [
    { view: "workspace", label: "Model", icon: Box },
    { view: "artifacts", label: "Files", icon: FileText }
  ];

  function startRename(session: CadSessionListItem) {
    setEditingSessionId(session.id);
    setDraftTitle(session.title ?? "Untitled CAD session");
    setOpenMenuSessionId(null);
  }

  function submitRename(sessionId: string) {
    const title = draftTitle.trim();
    if (!title) return;
    onRename(sessionId, title);
    setEditingSessionId(null);
  }

  return (
    <aside className={open ? "session-rail open" : "session-rail"} aria-label="Sessions">
      <div className="session-rail-header">
        <strong>Sessions</strong>
        <button onClick={onCreateSession} disabled={busy} title="Create session">
          <Plus size={16} /> New
        </button>
      </div>
      <label className="search-field session-rail-search">
        <Search size={15} />
        <input
          aria-label="Search sessions"
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          placeholder="Search"
        />
      </label>
      <label className="archive-toggle">
        <input
          type="checkbox"
          checked={showArchived}
          onChange={(event) => onShowArchivedChange(event.target.checked)}
        />
        <span>Show archived</span>
      </label>
      <ol className="session-rail-list">
        {visibleSessions.map((session) => {
          const isEditing = editingSessionId === session.id;
          const menuOpen = openMenuSessionId === session.id;
          return (
            <li className={session.id === activeSessionId ? "session-rail-item active" : "session-rail-item"} key={session.id}>
              {isEditing ? (
                <form
                  className="session-rail-rename"
                  onSubmit={(event) => {
                    event.preventDefault();
                    submitRename(session.id);
                  }}
                >
                  <input
                    aria-label={`Rename session ${session.id}`}
                    autoFocus
                    value={draftTitle}
                    onChange={(event) => setDraftTitle(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Escape") setEditingSessionId(null);
                    }}
                  />
                  <button
                    aria-label={`Save session title ${session.id}`}
                    disabled={busy || !draftTitle.trim()}
                    title="Save session title"
                    type="submit"
                  >
                    <Check size={15} />
                  </button>
                  <button
                    aria-label={`Cancel rename ${session.id}`}
                    onClick={() => setEditingSessionId(null)}
                    title="Cancel rename"
                    type="button"
                  >
                    <X size={15} />
                  </button>
                </form>
              ) : (
                <>
                  <button
                    className="session-rail-row"
                    disabled={busy || session.id === activeSessionId}
                    onClick={() => {
                      onOpenSession(session.id);
                      onClose();
                    }}
                    title={session.title ?? "Untitled CAD session"}
                  >
                    <span className="session-rail-title">{session.title ?? "Untitled CAD session"}</span>
                    <span className="session-rail-meta">
                      <Clock size={11} /> {formatRelativeDate(session.updatedAt)}
                    </span>
                    <span className="session-rail-facts">
                      <small>{session.revisionCount} rev</small>
                      <small>{session.artifactCount} files</small>
                      {session.archived ? <small><Archive size={11} /> archived</small> : null}
                      {!session.archived && session.status !== "idle" ? <small>{session.status}</small> : null}
                    </span>
                  </button>
                  <div className="session-rail-actions">
                    <button
                      aria-label={`Session actions for ${session.title ?? session.id}`}
                      className="session-rail-menu-button"
                      disabled={busy}
                      onClick={() => setOpenMenuSessionId(menuOpen ? null : session.id)}
                      title="Session actions"
                    >
                      <MoreVertical size={15} />
                    </button>
                    {menuOpen ? (
                      <div className="session-rail-menu" role="menu">
                        <button role="menuitem" onClick={() => startRename(session)} disabled={busy}>
                          <Edit3 size={15} /> Rename
                        </button>
                        <button
                          role="menuitem"
                          onClick={() => {
                            setOpenMenuSessionId(null);
                            onDuplicate(session.id);
                          }}
                          disabled={busy}
                        >
                          <Copy size={15} /> Duplicate
                        </button>
                        <button
                          role="menuitem"
                          onClick={() => {
                            setOpenMenuSessionId(null);
                            onArchiveChange(session.id, !session.archived);
                          }}
                          disabled={busy}
                        >
                          {session.archived ? <ArchiveRestore size={15} /> : <Archive size={15} />}
                          {session.archived ? "Unarchive" : "Archive"}
                        </button>
                        <button
                          role="menuitem"
                          onClick={() => {
                            setOpenMenuSessionId(null);
                            onDelete(session.id);
                          }}
                          disabled={busy}
                        >
                          <Trash2 size={15} /> Delete
                        </button>
                      </div>
                    ) : null}
                  </div>
                </>
              )}
            </li>
          );
        })}
      </ol>
      {visibleSessions.length === 0 ? <p className="session-rail-empty">No sessions found.</p> : null}
      <nav className="session-rail-nav" aria-label="Workspace tools">
        {navItems.map((item) => {
          const Icon = item.icon;
          return (
            <button
              className={view === item.view ? "active" : ""}
              key={item.view}
              onClick={() => {
                onNavigate(item.view);
                onClose();
              }}
              title={item.label}
            >
              <Icon size={16} /> {item.label}
            </button>
          );
        })}
      </nav>
    </aside>
  );
}

function sessionSortStamp(session: CadSessionListItem): string {
  return session.lastViewedAt ?? session.updatedAt;
}

function formatRelativeDate(value: string): string {
  const date = new Date(value);
  const diffMs = Date.now() - date.getTime();
  const diffMinutes = Math.max(0, Math.floor(diffMs / 60000));
  if (diffMinutes < 1) return "now";
  if (diffMinutes < 60) return `${diffMinutes}m`;
  const diffHours = Math.floor(diffMinutes / 60);
  if (diffHours < 24) return `${diffHours}h`;
  const diffDays = Math.floor(diffHours / 24);
  if (diffDays < 7) return `${diffDays}d`;
  return date.toLocaleDateString();
}
