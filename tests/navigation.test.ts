import assert from "node:assert/strict";
import test from "node:test";
import { toHistoryPath } from "../ui/src/navigation";

test("UI history paths can be derived from relative session URLs", () => {
  assert.equal(
    toHistoryPath("/sessions/226e5e5d-7fee-405f-837d-1b6cebb07f2b", "http://127.0.0.1:5173/"),
    "/sessions/226e5e5d-7fee-405f-837d-1b6cebb07f2b"
  );
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
