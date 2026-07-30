use crate::agent_adapter::AgentAdapterRunInput;
use crate::protocol::CadSourceLanguage;
use serde_json::{json, Value};
use std::path::Path;

pub(super) fn build_thread_start_params(cwd: &Path) -> Value {
    json!({
        "approvalPolicy": "never",
        "cwd": cwd,
        "personality": "pragmatic",
        "sandbox": "workspace-write",
        "serviceName": "cadastrophe-tauri-backend",
        "sessionStartSource": "startup"
    })
}

pub(super) fn build_turn_start_params(
    thread_id: &str,
    prompt: &str,
    cwd: &Path,
    app_data_dir: &Path,
) -> Value {
    json!({
        "threadId": thread_id,
        "input": [
            {
                "type": "text",
                "text": prompt,
                "text_elements": []
            }
        ],
        "personality": "pragmatic",
        "approvalPolicy": "never",
        "cwd": cwd,
        "sandboxPolicy": {
            "type": "workspaceWrite",
            "writableRoots": [app_data_dir],
            "networkAccess": false
        }
    })
}

pub(super) fn build_cad_prompt(input: &AgentAdapterRunInput) -> String {
    let latest_failure_report = input
        .latest_workflow_failure_report
        .as_ref()
        .map(|report| serde_json::to_string_pretty(report).unwrap_or_else(|_| report.to_string()))
        .unwrap_or_else(|| "null".to_string());
    let app_data_dir = shell_quote_path(&input.app_data_dir);
    format!(
        r#"You are the Cadastrophe Milestone 3 workflow agent.

Drive this Cadastrophe CAD request through the repository CLI workflow. Do not return a standalone JSON source object. The CLI tools own persisted plan, revision, preview, finalization, structural, and VLM workflow state.

User request:
{prompt}

Required order:
1. Inspect current state with `cadastrophe-session-state --app-data-dir {app_data_dir} --session {session_id}` when useful, especially on retries.
2. Create a CadModelPlan JSON file under the Cadastrophe app data directory, then commit it with `cadastrophe-plan-commit --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --plan <file>`.
3. Do not call `cadastrophe-source-apply`, `cadastrophe-preview-render`, `cadastrophe-evaluate-structural`, `cadastrophe-finalize`, or `cadastrophe-vlm-submit` until the plan commit succeeds for this run.
4. Write complete OpenSCAD source to a file under the Cadastrophe app data directory, then call `cadastrophe-source-apply --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --source <file> --language openscad`.
5. Render and inspect diagnostics with `cadastrophe-preview-render --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --revision <revision_id>`.
6. If runtime diagnostics contain errors, explain the cause briefly, repair the source, and repeat source apply then preview render.
7. When runtime diagnostics pass, call `cadastrophe-finalize --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --revision <revision_id>`.
8. Finalize runs the deterministic structural anchor. If it returns a `failure_report` or `next_action`/`nextAction` of `outer_loop_refine_source`, do not call VLM. Include that failure report in the next plan/source attempt and repeat from plan commit.
9. If finalize returns a `cadastrophe.vlm_judge.v1` contract or pending VLM state, hand that exact contract to a separate subagent using the `cadastrophe-vlm-judge` skill and request strict JSON only.
10. Save the judge JSON under the Cadastrophe app data directory, then submit it with `cadastrophe-vlm-submit --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --artifact <artifact_id> --report <file>`.
11. If VLM submit fails, use its `failure_report` for outer-loop refinement and repeat from plan commit. If it passes, finish the run with a concise assistant message naming the final revision/artifact.

CLI contract reminders:
- JSON output is the default; use `--pretty` only for human inspection.
- Always pass `--app-data-dir {app_data_dir}` to Cadastrophe CLI commands. The UI and CLI must operate on the same SQLite database and artifacts.
- Treat non-zero exits and JSON error envelopes as authoritative.
- Preserve command outputs that include `next_action`, `nextAction`, `diagnostics`, `failure_report`, `failureReport`, `artifact_paths`, `artifactPaths`, `contract_type`, or `contractType`; these fields are shown in the UI run log.
- A retry must inspect previous workflow failures via `cadastrophe-session-state --app-data-dir {app_data_dir} --session {session_id}` and carry the latest structural or VLM failure report into the new attempt.

OpenSCAD constraints for the current Cadastrophe openscad-wasm runtime:
- Use OpenSCAD CSG normally, including cube, sphere, cylinder, translate, rotate, union, and difference.
- Include simple numeric parameters with comments like `width = 40; // @param min=8 max=80 step=1 label=Width` when useful.
- Include a `// @main_component <name>` header matching the committed plan's `mainComponent.name`.
- Do not include file IO, `include`, `use`, or host-dependent paths.

Cadastrophe context:
- appDataDir: {app_data_dir}
- sessionId: {session_id}
- runId: {run_id}
- activeRevisionId: {revision_id}
- activeRevisionSourceLanguage: {source_language}
- latestWorkflowFailureReport:
```json
{latest_failure_report}
```
- activeRevisionSource:
```openscad
{source}
```
"#,
        prompt = input.prompt,
        app_data_dir = app_data_dir,
        session_id = input.session_id,
        run_id = input.run_id,
        revision_id = input.revision_id.as_deref().unwrap_or(""),
        latest_failure_report = latest_failure_report,
        source_language = input
            .revision_source_language
            .as_ref()
            .map(|language| match language {
                CadSourceLanguage::Openscad => "openscad",
                CadSourceLanguage::Cadquery => "cadquery",
                CadSourceLanguage::FreecadPython => "freecad-python",
                CadSourceLanguage::CadastropheIr => "cadastrophe-ir",
            })
            .unwrap_or(""),
        source = input.revision_source.as_deref().unwrap_or("")
    )
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}
