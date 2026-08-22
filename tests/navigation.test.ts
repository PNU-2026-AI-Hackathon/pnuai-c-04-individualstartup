import assert from "node:assert/strict";
import test from "node:test";
import {
  sessionIdFromUrl,
  sessionPathWithView,
  toHistoryPath,
  workspaceViewFromUrl
} from "../ui/src/navigation";

test("UI history paths can be derived from relative session URLs", () => {
  assert.equal(
    toHistoryPath("/sessions/226e5e5d-7fee-405f-837d-1b6cebb07f2b", "http://127.0.0.1:5173/"),
    "/sessions/226e5e5d-7fee-405f-837d-1b6cebb07f2b"
  );
});

test("workspace tab paths preserve session id and supported view", () => {
  const path = sessionPathWithView("session-1", "logs");
  assert.equal(path, "/sessions/session-1?view=logs");
  assert.equal(sessionIdFromUrl(path, "http://127.0.0.1:5173/"), "session-1");
  assert.equal(workspaceViewFromUrl(path, "http://127.0.0.1:5173/"), "logs");
});

test("workspace tab parsing falls back to workspace for unknown views", () => {
  assert.equal(
    workspaceViewFromUrl("/sessions/session-1?view=admin", "http://127.0.0.1:5173/"),
    "workspace"
  );
});

test("settings view is addressable within a session", () => {
  const path = sessionPathWithView("session-1", "settings");
  assert.equal(path, "/sessions/session-1?view=settings");
  assert.equal(workspaceViewFromUrl(path, "http://127.0.0.1:5173/"), "settings");
});

test("UI history paths preserve query parameters from absolute desktop session URLs", () => {
  assert.equal(
    toHistoryPath(
      "tauri://localhost/sessions/session-1?source=recent",
      "http://127.0.0.1:5173/"
    ),
    "/sessions/session-1?source=recent"
  );
});
