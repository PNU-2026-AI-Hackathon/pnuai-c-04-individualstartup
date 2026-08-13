## DFM 검증기 개발 계획서

### 1. 목표

PrusaSlicer 기반 3D 프린팅 DFM 검증기를 추가한다. 에이전트가 cadastrophe-finalize를 호출하면 동일한 STL에 대해 기하 검
증과 DFM 검증을 함께 수행하고 결과를 finalization 응답에 포함한다.

STL Export
    ├─ 기하 검증 Sidecar
    └─ PrusaSlicer DFM 검증
        ↓
두 검증 통과 → 9-view/VLM 검증
하나라도 불합격 → failureReport와 함께 모델 수정

### 2. 백엔드 구현

- dfm.rs와 cadastrophe.dfm_report.v1 계약을 추가한다.
- 앱 설정에 저장된 PrusaSlicer 절대경로로 다음 subprocess를 실행한다.

<prusaslicer-path> --load <profile.ini> --export-gcode <model.stl>

- exit code, stdout, stderr와 생성된 G-code를 수집한다.
- stdout 진단을 passed, checks, diagnostics, profileHash, gcodeArtifactId로 구조화한다.
- 구조 또는 DFM 검증이 불합격이면 두 보고서를 모두 반환하고 nextAction을 outer_loop_refine_source로 설정한다.
- 두 검증을 모두 통과한 경우에만 9-view/VLM 단계로 진행한다.
- 실행 실패, 비정상 종료, 잘못된 profile, G-code 미생성은 즉시 오류 처리한다.

### 3. PrusaSlicer 실행 파일 설정

앱의 Settings 화면에 PrusaSlicer executable 설정을 추가한다.

- 파일 선택기를 통해 실행 파일의 절대경로 지정
- macOS .app 선택 시 내부 CLI 경로 선택 안내 또는 자동 변환
- 저장 전 절대경로, 파일 존재 여부, 실행 가능 여부 검증
- <path> --version 실행으로 PrusaSlicer 호환성 확인
- 검증 성공 시 버전과 상태를 UI에 표시
- 설정값은 로컬 app_kv 또는 전용 설정 저장소에 영속화
- subprocess는 셸을 거치지 않고 저장된 경로를 직접 실행
- 경로 누락·손상 시 PATH나 alias로 우회하지 않고 설정 오류를 명확히 반환

### 4. Profile UI/UX

profile.ini 기반의 검색 가능한 설정 편집 화면을 제공한다.

- Printer, Filament, Quality, Support, Speed 등의 항목 분류
- 숫자, Boolean, enum, %, 다중 값에 맞는 입력 컴포넌트
- 전체 설정을 수정할 수 있는 고급 key/value 편집 화면
- INI 가져오기·내보내기와 기본값 복원
- 저장 전 구문 및 필수 설정 검증
- DFM 결과에 사용한 profile hash와 주요 설정 표시

### 5. 저장 및 테스트

- workflow_outer_iterations에 dfm_report_json을 추가한다.
- pending VLM 상태에도 DFM 보고서를 연결한다.
- STL, G-code, DFM 보고서에 revision/profile hash를 기록한다.
- 정상 슬라이싱, DFM 불합격, binary 미설정·손상, 잘못된 profile, G-code 미생성 테스트를 작성한다.
- 설정 저장·재시작 복구·실행 파일 변경 UI 테스트를 추가한다.
