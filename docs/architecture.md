# Cadastrophe Architecture

Last updated: 2026-07-28

이 문서는 현재 앱의 실제 구조를 설명한다. Milestone별 완료 기준과 다음 작업은
`docs/development_plan.md`, 작업 이력은 `docs/progress.md`에서 관리한다.

## 1. Product Shape

Cadastrophe는 Tauri desktop 앱으로 배포하는 로컬 Text-to-CAD 작업공간이다. React
UI는 desktop WebView 안에서 첫 사용자 접점이 되고, Rust/Tauri backend가 session
state, agent run, artifact, preview/export runtime을 소유한다.

현재 제품 루프는 다음과 같다.

```text
User
  -> Tauri WebView
  -> React/Vite UI
  -> CadBackendClient
  -> Tauri IPC command / cad_bridge_event
  -> Rust AgentGateway
  -> AgentAdapter
  -> Rust SessionService
  -> Rust OpenSCAD preview/export runtime
  -> WebView snapshot update
```

브라우저 단독 HTTP bridge와 TypeScript backend는 더 이상 runtime target이 아니다.
개발 중 UI asset은 Vite가 제공하지만, 앱 동작 기준 transport는 Tauri IPC와 Tauri
event다.

## 2. Runtime Targets

현재 확정 runtime target은 desktop 앱 하나다.

| Runtime | Role |
| --- | --- |
| Tauri/Rust app | Canonical backend. Tauri command/event surface, session/run/artifact state, real Codex process adapter, preview/export runtime을 소유한다. |
| React/Vite UI | Desktop WebView workspace UI. `@tauri-apps/api`를 통해 backend command와 `cad_bridge_event`를 사용한다. |

Desktop run의 기본 adapter는 real `CodexAgentAdapter`다.
`CADASTROPHE_AGENT_ADAPTER=fake`일 때만 deterministic fake adapter로 동작한다.

## 3. Protocol

Rust DTO는 `src-tauri/src/protocol.rs`가 canonical source다. UI compile-time 타입은
`ui/src/protocol.ts`에 mirror되어 Tauri command/event payload shape를 표현한다.

주요 state object는 다음과 같다.

| Object | Meaning |
| --- | --- |
| `CadSession` | session metadata, active revision, selected runtime, UI connection/status summary. |
| `CadRevision` | CAD source, parameters, diagnostics, artifacts, revision-scoped user events. |
| `CadArtifact` | preview mesh, STL, metadata 같은 revision output과 read/export 경로. |
| `CadConversationMessage` | UI와 agent 사이의 user/assistant/system/tool 메시지 기록. |
| `CadAgentRun` | agent run lifecycle: queued, running, completed, failed, cancelled. |
| `CadSessionState` | UI와 agent가 받는 full snapshot. session, active revision, messages, conversation, runs를 포함한다. |
| `CadBridgeEvent` | 모든 mutation 이후 UI로 전파되는 level-triggered full snapshot event. |

설계 원칙은 "부분 delta가 아니라 full snapshot을 반복 전송"하는 것이다. UI는 missed
event 이후에도 `getSessionState` 또는 다음 snapshot으로 복구할 수 있다.

## 4. Tauri/Rust Backend

| Path | Responsibility |
| --- | --- |
| `src-tauri/src/lib.rs` | Tauri command 등록, app state wiring, event forwarding, smoke path. |
| `src-tauri/src/protocol.rs` | Canonical command/result/state/event DTO. |
| `src-tauri/src/session_service.rs` | session/revision/artifact/message/conversation/run state authority. |
| `src-tauri/src/agent_gateway.rs` | session별 run serialization, cancellation, adapter event application. |
| `src-tauri/src/agent_adapter.rs` | Agent adapter trait와 event/input contract. |
| `src-tauri/src/codex_agent_adapter.rs` | real Codex app-server event mapping과 structured OpenSCAD result parsing. |
| `src-tauri/src/codex_process_client.rs` | `codex app-server --listen stdio://` child process management, PATH resolution, timeout/interrupt. |
| `src-tauri/src/fake_agent_adapter.rs` | deterministic desktop smoke/test adapter. |
| `src-tauri/src/runtime.rs` | Rust OpenSCAD MVP preview/export logic. |
| `src-tauri/capabilities/default.json` | Tauri permission/capability policy for app commands and events. |

GUI launch 환경은 interactive shell의 PATH를 그대로 받지 않기 때문에, Codex process
client는 PATH와 일반적인 macOS Homebrew/npm 위치를 함께 탐색한다.

## 5. Web UI

`ui/src/App.tsx`는 첫 화면부터 session workspace를 연다. landing page는 없다.

| Path | Responsibility |
| --- | --- |
| `ui/src/backendClient.ts` | Tauri IPC command와 `cad_bridge_event` listener를 감싼 `CadBackendClient`. |
| `ui/src/protocol.ts` | Rust protocol mirror 타입. UI compile-time contract 용도다. |
| `ui/src/App.tsx` | session boot, prompt composer, conversation/run state, source editor, parameters, diagnostics, timeline, export UI. |
| `ui/src/MeshPreview.tsx` | Three.js preview mesh renderer. OrbitControls로 drag orbit과 wheel zoom을 지원한다. |
| `ui/src/navigation.ts` | `/`와 `/sessions/{id}` navigation helper. |
| `ui/src/styles.css` | workspace layout, preview bounds, responsive UI styling. |

UI boot 규칙은 다음과 같다.

- `/sessions/{id}`로 열리면 해당 session을 load하고 viewed state로 표시한다.
- `/`로 열리면 current session을 재사용하고, 없으면 새 session을 만든다.
- source draft가 dirty일 때 외부 revision이 들어오면 자동 덮어쓰기 대신 conflict
  state를 표시한다.
- preview mesh는 `read_artifact` Tauri command를 통해 artifact contents를 읽는다.

## 6. Agent Flow

Agent run은 UI prompt composer에서 시작된다.

1. UI가 `create_agent_run` Tauri command를 호출한다.
2. `AgentGateway`가 user conversation message와 queued run을 생성한다.
3. session별 queue가 같은 session의 run을 순서대로 실행한다.
4. adapter event는 assistant/tool/system message, active step, source update,
   preview render, failure, cancellation으로 매핑된다.
5. 각 mutation은 full `CadBridgeEvent` snapshot으로 UI에 전파된다.

## 7. Artifact And Runtime Model

Milestone 1의 필수 CAD runtime은 OpenSCAD MVP다.

- 지원 source language: `openscad`
- preview artifact: `preview-mesh` JSON
- export artifact: `stl`, `metadata`
- 지원 subset: `cube`, `sphere`, `cylinder`, 단순 `translate`
- parameter extraction: OpenSCAD assignment와 `// @param` metadata

Artifact는 app data directory의 `artifacts` 아래에 기록된다. Artifact metadata는
session state에 index되고, UI는 Tauri command를 통해 내용을 읽는다.

## 8. Removed TypeScript Backend

이전 TypeScript backend runtime은 제거됐다. 삭제된 surface는 다음과 같다.

- `src/server/**` TypeScript session service, agent gateway, fake adapter, OpenSCAD
  runtime, MCP stdio server, HTTP/WebSocket bridge.
- `/api/**`, `/artifacts/**`, `/ws` browser bridge routes.
- `dev:server` script와 MCP/HTTP bridge 전용 npm dependencies.

남은 TypeScript 코드는 React UI, UI protocol mirror, UI-oriented tests, Vite config에
한정한다.

## 9. Verification Surface

현재 문서가 기준으로 삼는 verification command는 다음이다.

```sh
npm run check
npm test
npm run test:rust
npm run build
npm run smoke:tauri
npm run build:tauri
npm run smoke:codex -- "Create a simple OpenSCAD sphere with radius 6."
```

`smoke:codex`는 로컬에 설치되고 인증된 Codex CLI에 의존한다. release 전에는
packaged `.app`에서 물리적으로 Run button을 다시 눌러 desktop 환경의 Codex 실행
경로를 확인해야 한다.
