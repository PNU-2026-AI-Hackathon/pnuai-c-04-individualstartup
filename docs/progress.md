# Cadastrophe Progress

Last updated: 2026-07-28

이 문서는 작업 이력을 시간순으로 한 줄씩 추가하는 로그다. 자세한 구조 설명은
`docs/architecture.md`, Milestone 범위와 완료 여부는 `docs/development_plan.md`에
기록한다.

Format:

```text
YYYY/MM/DD HH:mm 작업 내용.
```

## Log

- 2026/07/28 Tauri desktop 앱 방향 확정: TypeScript backend, MCP/HTTP/WebSocket bridge, `dev:server` script, 관련 npm dependencies 제거.
- 2026/07/28 TypeScript protocol mirror를 `ui/src/protocol.ts`로 이동하고 UI backend client를 Tauri IPC/event 전용으로 축소.
- 2026/07/28 16:24 docs 문서 역할 개편: architecture는 실제 앱 구조, development_plan은 Milestone 현황, progress는 시간순 작업 로그로 정리.
- 2026/07/28 16:21 처음 접속시 렌더링 화면 무한 확장 문제 디버깅 및 preview container/Three.js mount 조건 수정.
- 2026/07/28 Codex desktop preview mismatch 수정: Rust preview runtime이 parameter placeholder 대신 OpenSCAD source에서 mesh를 생성하도록 변경.
- 2026/07/28 Finder/Tauri GUI launch에서 codex를 찾지 못하는 문제 수정: PATH와 macOS Homebrew/npm 경로를 함께 탐색.
- 2026/07/28 real Codex adapter를 desktop 기본 agent path로 전환하고 fake adapter는 `CADASTROPHE_AGENT_ADAPTER=fake` opt-in으로 변경.
- 2026/07/28 Codex CLI 0.142.3 app-server event shape 대응, structured OpenSCAD JSON parsing, timeout interrupt 추가.
- 2026/07/28 legacy Approval decision panel 및 decision-channel API surface 제거, agent prompt composer를 사용자 피드백 채널로 통합.
- 2026/07/28 Tauri event listen ACL 문제 수정: main window capability에 `core:event:default` 추가.
- 2026/07/28 Tauri `create_agent_run` synchronous command crash 수정: Rust `AgentGateway` background work를 Tauri async runtime으로 spawn.
- 2026/07/28 Rust `AgentGateway` session별 run 직렬화와 순서 보장 regression test 추가.
- 2026/07/28 TypeScript backend와 React UI에 agent run create/list/get/cancel contract와 WebSocket snapshot 반영.
- 2026/07/28 React UI에 prompt composer, conversation timeline, run/tool/failure/cancel/retry 상태 표시 추가.
- 2026/07/28 `CadBackendClient`로 HTTP/WebSocket transport와 Tauri IPC/event transport 추상화.
- 2026/07/28 Three.js `OrbitControls`로 preview drag orbit 및 wheel zoom 지원.
- 2026/07/28 TypeScript reference backend에 session/revision/source/preview/export/user message state와 MCP/HTTP bridge 구현.
- 2026/07/28 OpenSCAD MVP runtime 구현: `cube`, `sphere`, `cylinder`, 단순 `translate`, preview mesh, STL/metadata export.
