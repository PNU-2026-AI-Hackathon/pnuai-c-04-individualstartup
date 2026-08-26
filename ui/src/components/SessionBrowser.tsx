import { useState } from "react";
import {
  Archive,
  ArchiveRestore,
  Check,
  Clock,
  Copy,
  Edit3,
  Search,
  Trash2,
  X
} from "lucide-react";
import type { CadSessionListItem } from "../protocol";

export function SessionBrowser({
  sessions,
  activeSessionId,
  query,
  searchFields,
  busy,
  onQueryChange,
  onOpen,
  onArchiveChange,
  onRename,
  onDuplicate,
  onDelete
}: {
  sessions: CadSessionListItem[];
  activeSessionId: string;
  query: string;
  searchFields: string[];
  busy: boolean;
  onQueryChange: (query: string) => void;
  onOpen: (sessionId: string) => void;
  onArchiveChange: (sessionId: string, archived: boolean) => void;
  onRename: (sessionId: string, title: string) => void;
  onDuplicate: (sessionId: string) => void;
  onDelete: (sessionId: string) => void;
}) {
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");

  function startRename(session: CadSessionListItem) {
    setEditingSessionId(session.id);
    setDraftTitle(session.title ?? "Untitled CAD session");
  }

  function submitRename(sessionId: string) {
    const title = draftTitle.trim();
    if (!title) return;
    onRename(sessionId, title);
    setEditingSessionId(null);
  }

  return (
    <section className="management-view" data-testid="session-browser">
      <div className="management-toolbar">
        <label className="search-field">
          <Search size={16} />
          <input
            aria-label={`Search sessions by ${searchFields.join(", ") || "title and source"}`}
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Search sessions"
          />
        </label>
      </div>
      <ol className="session-list">
        {sessions.map((session) => {
          const isEditing = editingSessionId === session.id;
          return (
            <li className={session.id === activeSessionId ? "active" : ""} key={session.id}>
              <div className="session-list-main">
                {isEditing ? (
                  <div className="rename-row">
                    <input
                      aria-label={`Rename session ${session.id}`}
                      value={draftTitle}
                      onChange={(event) => setDraftTitle(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") submitRename(session.id);
                        if (event.key === "Escape") setEditingSessionId(null);
                      }}
                    />
                    <button
                      aria-label={`Save session title ${session.id}`}
                      disabled={busy || !draftTitle.trim()}
                      onClick={() => submitRename(session.id)}
                      title="Save session title"
                    >
                      <Check size={16} />
                    </button>
                    <button
                      aria-label={`Cancel rename ${session.id}`}
                      onClick={() => setEditingSessionId(null)}
                      title="Cancel rename"
                    >
                      <X size={16} />
                    </button>
                  </div>
                ) : (
                  <button
                    className="session-title-button"
                    onClick={() => onOpen(session.id)}
                    disabled={busy}
                    title="Open session"
                  >
                    {session.title ?? "Untitled CAD session"}
                  </button>
                )}
                <span>{session.id}</span>
                <div className="session-list-meta">
                  <small><Clock size={13} /> {new Date(session.updatedAt).toLocaleString()}</small>
                  <small>{session.revisionCount} revisions</small>
                  <small>{session.artifactCount} artifacts</small>
                  {session.archived ? <small><Archive size={13} /> archived</small> : null}
                </div>
                {session.activeRevision ? (
                  <div className="active-revision-summary">
                    <code>{session.activeRevision.id.slice(0, 8)}</code>
                    <span>{session.activeRevision.sourceLanguage}</span>
                    <span>{session.activeRevision.artifactCount} artifacts</span>
                    <span>{session.activeRevision.sourceHash.slice(0, 10)}</span>
                  </div>
                ) : null}
              </div>
              <div className="session-list-actions">
                <button onClick={() => startRename(session)} disabled={busy} title="Rename session">
                  <Edit3 size={16} /> Rename
                </button>
                <button onClick={() => onDuplicate(session.id)} disabled={busy} title="Duplicate session">
                  <Copy size={16} /> Duplicate
                </button>
                <button
                  onClick={() => onArchiveChange(session.id, !session.archived)}
                  disabled={busy}
                  title={session.archived ? "Unarchive session" : "Archive session"}
                >
                  {session.archived ? <ArchiveRestore size={16} /> : <Archive size={16} />}
                  {session.archived ? "Unarchive" : "Archive"}
                </button>
                <button
                  onClick={() => {
                    console.info("[cadgen-ax:delete-session] browser delete clicked", { sessionId: session.id });
                    onDelete(session.id);
                  }}
                  disabled={busy}
                  title="Delete session"
                >
                  <Trash2 size={16} /> Delete
                </button>
              </div>
            </li>
          );
        })}
      </ol>
      {sessions.length === 0 ? <p className="empty-state">No sessions found.</p> : null}
    </section>
  );
}
