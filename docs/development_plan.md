# Cadastrophe Development Plan

Last updated: 2026-07-28

이 문서는 Milestone 범위와 완료 여부를 관리한다. 실제 앱 구조는
`docs/architecture.md`, 시간순 작업 이력은 `docs/progress.md`에서 관리한다.

## Milestone Summary

| Milestone | Status | Scope |
| --- | --- | --- |
| Milestone 1 | Complete | Tauri desktop local CAD workspace, agent run loop, OpenSCAD preview/export MVP, Rust backend. |
| Milestone 2 | Planned | Session, artifact, revision, and agent chat log management. |
| Milestone 3 | Unplanned | Distribution, persistence hardening, runtime expansion, packaging policy. |

## Milestone 1: Tauri Desktop CAD Workspace

Status: Complete.

Milestone 1의 목표는 사용자가 Tauri desktop 앱에서 바로 CAD 작업을 시작하고,
agent run이 session state를 갱신하며, preview/export artifact와 conversation/run
상태가 같은 workspace 안에서 동기화되는 것이다.

완료된 범위:

- React/Vite workspace가 첫 화면으로 열린다.
- `/`는 current session을 재사용하거나 새 session을 만들고, `/sessions/{id}`는
  특정 session을 연다.
- Tauri/Rust backend는 IPC command와 `cad_bridge_event`로
  session/revision/artifact/message/conversation/run state를 제공한다.
- TypeScript backend, MCP stdio surface, HTTP/WebSocket bridge는 제거됐다.
- UI prompt composer에서 agent run을 시작하고, run 상태, tool step, 실패, 취소,
  retry 가능 상태를 conversation과 함께 표시한다.
- `AgentGateway`는 같은 session의 run을 직렬화한다.
- Desktop backend는 real `CodexAgentAdapter`를 기본값으로 사용한다.
- Deterministic fake adapter는 명시적인 test/smoke 경로에 남겨 둔다.
- OpenSCAD MVP runtime은 `cube`, `sphere`, `cylinder`, 단순 `translate` subset을
  preview mesh로 렌더링하고 STL/metadata export를 제공한다.
- Preview는 Three.js와 OrbitControls를 사용해 drag orbit과 wheel zoom을 지원한다.
- Source editor, parameters, diagnostics, revision timeline, artifact export link가
  같은 workspace 안에 있다.
- Source draft conflict protection이 있다.
- 첫 load preview 무한 확장 문제와 mobile viewport metadata 누락을 수정했다.

Verification command set:

```sh
npm run check
npm test
npm run test:rust
npm run build
npm run smoke:tauri
npm run build:tauri
npm run smoke:codex -- "Create a simple OpenSCAD sphere with radius 6."
```

최근 검증 결과:

- `npm run check`: passed.
- `npm test`: passed, 3 tests.
- `npm run test:rust`: passed, 16 tests.
- `npm run build`: passed with Vite large chunk warning.
- `npm run build:tauri`: passed and produced macOS app/dmg artifacts.
- `npm run smoke:tauri`: not rerun after TypeScript backend removal.
- `npm run smoke:codex -- "Create a simple OpenSCAD sphere with radius 6."`: not
  rerun after TypeScript backend removal.

남은 release 전 확인:

- Packaged `.app`에서 실제 Run button 클릭을 한 번 더 수동 확인한다.
- Codex CLI 설치/인증 실패, timeout, retry diagnostics를 더 명확히 만든다.

## Milestone 2: Session, Artifact, Revision, Agent Chat Log Management

Status: Planned.

Milestone 2의 중심은 생성된 CAD 작업 기록을 다루는 관리 기능이다. Milestone 1이
"한 session 안의 생성 루프"를 닫았다면, Milestone 2는 "여러 session과 결과물을
찾고, 비교하고, 보존하고, 정리하는 경험"을 만든다.

예상 범위:

- Session list, search, rename, duplicate, archive/delete.
- Session persistence across app restart.
- Revision list 개선, revision diff/restore, active revision 변경.
- Artifact browser, artifact metadata detail, artifact delete, export 재다운로드.
- Agent chat log viewer, run별 log grouping, tool event detail, failure diagnostics.
- Run cancellation/retry 기록 보존.
- Storage layout과 migration 정책.
- UI navigation for session/revision/artifact/log management.

초기 완료 기준 초안:

- 앱 재시작 후 session, active revision, artifacts, conversation, agent runs가
  복원된다.
- 사용자는 session 목록에서 이전 작업을 열 수 있다.
- 사용자는 revision을 비교하고 이전 revision으로 되돌릴 수 있다.
- 사용자는 artifact 목록을 보고 필요한 export를 다시 열거나 삭제할 수 있다.
- 사용자는 agent chat log와 run/tool/failure 기록을 session 단위로 확인할 수 있다.
- Tauri/Rust backend의 persistence contract가 테스트로 검증된다.

## Future Work

아직 Milestone으로 확정하지 않은 후보:

- Rust preview runtime을 실제 OpenSCAD runtime 수준으로 끌어올리기.
- CadQuery/FreeCAD runtime adapter.
- STEP, 3MF, BRep-quality export.
- Packaged app signing, auto-update, release artifact retention.
- UI code splitting으로 Vite large chunk warning 정리.
- Cloud-hosted multi-tenant mode.
- User authentication and authorization.

## Out Of Scope For Now

- Chrome extension/native-host/page/voice/image workflow planes.
- Marketplace packaging policy.
- General-purpose collaboration.
- Cloud operations and account system.
