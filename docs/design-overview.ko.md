# Cadastrophe 설계 요약

Last updated: 2026-07-28

Cadastrophe는 Tauri desktop 앱으로 배포하는 로컬 text-to-CAD 작업공간이다.
사용자는 WebView UI에서 agent에게 CAD 작업을 요청하고, Rust backend는 session,
source revision, preview/export artifact, conversation, agent run 상태를 하나의
canonical state로 관리한다.

상세 구조는 `docs/architecture.md`, Milestone 현황은
`docs/development_plan.md`, 작업 이력은 `docs/progress.md`에 둔다.

## 현재 구조

```text
User
  -> Tauri WebView
  -> React UI
  -> CadBackendClient
  -> Tauri IPC/event
  -> AgentGateway
  -> AgentAdapter
  -> Rust SessionService
  -> OpenSCAD preview/export runtime
  -> WebView snapshot update
```

## 핵심 결정

- Desktop WebView UI가 첫 화면이다. landing page나 CLI-first review URL 흐름이
  기본이 아니다.
- Browser-only HTTP bridge와 TypeScript backend는 runtime target에서 제거됐다.
- Canonical state는 Rust `SessionService`가 가진다. UI는 full snapshot을 받고
  mutation은 backend command로 제출한다.
- Agent와 UI의 대화는 별도 approval panel이 아니라 prompt composer와 conversation
  log를 중심으로 처리한다.
- 같은 session의 agent run은 직렬화해 concurrent CAD state mutation을 막는다.
- Desktop backend는 real Codex adapter를 기본으로 사용하고, fake adapter는
  deterministic test/smoke 용도로만 opt-in한다.
- Preview는 Three.js mesh viewer이며 drag orbit과 wheel zoom을 지원한다.

## 현재 범위

- React/Vite workspace UI.
- Tauri/Rust desktop backend with IPC commands and `cad_bridge_event` snapshots.
- Agent run lifecycle: queued, running, completed, failed, cancelled.
- Conversation and run state in `CadSessionState`.
- OpenSCAD MVP runtime for `cube`, `sphere`, `cylinder`, simple `translate`.
- Preview mesh JSON, STL export, metadata export.

## 다음 방향

Milestone 2는 session, artifact, revision, agent chat log 관리가 중심이다.
예상 작업은 persistence, session list/search/rename/archive, revision diff/restore,
artifact browser/delete/re-download, run/tool/failure log viewer다.
