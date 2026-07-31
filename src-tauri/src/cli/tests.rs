use super::model_commands::{plan_commit, preview_render, source_apply};
use super::workflow_commands::{finalize, vlm_submit};
use super::*;
use crate::protocol::CreateCadSessionInput;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
#[test]
fn finalize_requires_committed_plan() {
    let app_data_dir = temp_app_data_dir("finalize-requires-plan");
    let service = sqlite_service(&app_data_dir);
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let revision_id = create_test_revision(&service, &created.session_id);
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "create model".to_string(),
            Some(revision_id.clone()),
            Some("test".to_string()),
            None,
        )
        .unwrap();

    let error = finalize(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("revision", &revision_id),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap_err();

    assert_eq!(error.code, "precondition_failed");
    assert!(error.message.contains("cadastrophe-plan-commit"));
}

#[cfg(unix)]
#[test]
fn finalize_structural_fail_appends_outer_iteration_without_pending_vlm() {
    let app_data_dir = temp_app_data_dir("finalize-structural-fail");
    let service = sqlite_service(&app_data_dir);
    let setup = setup_run_with_plan(&service);
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-fail",
        &structural_report_json(&setup.run_id, &setup.revision_id, false),
    );

    let output = finalize(
        &args([
            ("session", &setup.session_id),
            ("run", &setup.run_id),
            ("revision", &setup.revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(
        output.data["next_action"].as_str(),
        Some("outer_loop_refine_source")
    );
    assert_eq!(
        output.data["failure_report"]["contractType"].as_str(),
        Some("cadastrophe.failure_report.v1")
    );
    let state = service.get_session_state(&setup.session_id).unwrap();
    assert_eq!(state.workflow.pending_vlm.len(), 0);
    assert_eq!(state.workflow.outer_iterations.len(), 1);
    assert!(!state.workflow.outer_iterations[0].passed);
    assert!(state.workflow.outer_iterations[0].vlm_report.is_none());
}

#[cfg(unix)]
#[test]
fn finalize_structural_pass_creates_pending_vlm_contract() {
    let app_data_dir = temp_app_data_dir("finalize-structural-pass");
    let service = sqlite_service(&app_data_dir);
    let setup = setup_run_with_plan(&service);
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-pass",
        &structural_report_json(&setup.run_id, &setup.revision_id, true),
    );
    let renderer_sidecar = fixture_renderer_sidecar(&app_data_dir, "renderer-pass");

    let output = finalize(
        &args([
            ("session", &setup.session_id),
            ("run", &setup.run_id),
            ("revision", &setup.revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
            ("renderer-sidecar", renderer_sidecar.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(output.data["next_action"].as_str(), Some("vlm_judge"));
    assert_eq!(
        output.data["vlmContract"]["contractType"].as_str(),
        Some("cadastrophe.vlm_judge.v1")
    );
    assert_eq!(
        output.data["vlmContract"]["renderedImages"]["available"].as_bool(),
        Some(true)
    );
    assert_eq!(
        output.data["vlmContract"]["renderedImages"]["format"].as_str(),
        Some("png")
    );
    let state = service.get_session_state(&setup.session_id).unwrap();
    assert_eq!(state.workflow.pending_vlm.len(), 1);
    assert_eq!(state.workflow.pending_vlm[0].run_id, setup.run_id);
    assert_eq!(state.workflow.outer_iterations.len(), 0);
    assert!(state
        .active_revision
        .as_ref()
        .unwrap()
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == CadArtifactKind::RenderImage));
}

#[cfg(unix)]
#[test]
fn cli_workflow_persists_required_tool_event_order() {
    let app_data_dir = temp_app_data_dir("workflow-event-order");
    let service = sqlite_service(&app_data_dir);
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let input_revision_id = created.state.session.active_revision_id.clone();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Create a workflow event order fixture.".to_string(),
            input_revision_id,
            Some("test".to_string()),
            None,
        )
        .unwrap();
    let plan_path = write_json_file(
        &app_data_dir,
        "plan.json",
        &serde_json::from_str(include_str!(
            "../../../fixtures/contracts/cad_model_plan.v1.json"
        ))
        .unwrap(),
    );
    let source_path = app_data_dir.join("source.scad");
    fs::write(
        &source_path,
        "// @main_component wall_bracket\ncube([3, 1, 2]);\n",
    )
    .unwrap();

    plan_commit(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("plan", plan_path.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();
    let source_output = source_apply(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("source", source_path.to_str().unwrap()),
            ("language", "openscad"),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();
    let revision_id = source_output.data["revisionId"]
        .as_str()
        .unwrap()
        .to_string();
    preview_render(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("revision", &revision_id),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-event-order-pass",
        &structural_report_json(&run.id, &revision_id, true),
    );
    let renderer_sidecar = fixture_renderer_sidecar(&app_data_dir, "renderer-event-order");
    finalize(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("revision", &revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
            ("renderer-sidecar", renderer_sidecar.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    let state = service.get_session_state(&created.session_id).unwrap();
    let completed_commands = state
        .agent_run_events
        .iter()
        .filter(|event| event.run_id == run.id)
        .filter(|event| event.event_type == CadAgentRunEventType::AgentToolCompleted)
        .filter_map(|event| event.payload.get("command").and_then(Value::as_str))
        .collect::<Vec<_>>();

    assert_eq!(
        completed_commands,
        vec![
            "cadastrophe-plan-commit",
            "cadastrophe-source-apply",
            "cadastrophe-preview-render",
            "cadastrophe-evaluate-structural",
            "cadastrophe-finalize",
        ]
    );
    assert_eq!(state.workflow.plans.len(), 1);
    assert_eq!(state.workflow.pending_vlm.len(), 1);
}

#[test]
fn preview_runtime_failure_records_source_repair_event_diagnostics() {
    let app_data_dir = temp_app_data_dir("preview-source-repair");
    let service = sqlite_service(&app_data_dir);
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let input_revision_id = created.state.session.active_revision_id.clone();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Create source that needs repair.".to_string(),
            input_revision_id,
            Some("test".to_string()),
            None,
        )
        .unwrap();
    let plan_path = write_json_file(
        &app_data_dir,
        "repair-plan.json",
        &serde_json::from_str(include_str!(
            "../../../fixtures/contracts/cad_model_plan.v1.json"
        ))
        .unwrap(),
    );
    let source_path = app_data_dir.join("invalid.scad");
    fs::write(
        &source_path,
        "// @main_component wall_bracket\nunsupported();\n",
    )
    .unwrap();

    plan_commit(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("plan", plan_path.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();
    let source_output = source_apply(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("source", source_path.to_str().unwrap()),
            ("language", "openscad"),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();
    let revision_id = source_output.data["revisionId"].as_str().unwrap();
    let preview_output = preview_render(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("revision", revision_id),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(preview_output.data["next_action"].as_str(), None);
    assert_eq!(
        preview_output.data["nextAction"].as_str(),
        Some("source_repair")
    );
    let state = service.get_session_state(&created.session_id).unwrap();
    let preview_event = state
        .agent_run_events
        .iter()
        .rev()
        .find(|event| {
            event.run_id == run.id
                && event.event_type == CadAgentRunEventType::AgentToolCompleted
                && event.payload.get("command").and_then(Value::as_str)
                    == Some("cadastrophe-preview-render")
        })
        .unwrap();
    assert_eq!(
        preview_event
            .payload
            .get("nextAction")
            .and_then(Value::as_str),
        Some("source_repair")
    );
    assert!(preview_event
        .payload
        .get("diagnostics")
        .and_then(|diagnostics| diagnostics.get("items"))
        .and_then(Value::as_array)
        .is_some_and(|items| items.iter().any(|item| {
            item.get("message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains("Current top level object is empty"))
        })));
}

#[cfg(unix)]
#[test]
fn vlm_submit_fail_and_pass_consume_pending_and_append_outer_iterations() {
    let app_data_dir = temp_app_data_dir("vlm-submit");
    let service = sqlite_service(&app_data_dir);
    let failed = setup_pending_vlm(&service, &app_data_dir, "fail");
    let fail_report = write_json_file(
        &app_data_dir,
        "vlm-fail.json",
        &json!({
            "contractType": "cadastrophe.vlm_judge_report.v1",
            "runId": failed.run_id,
            "artifactId": failed.artifact_id,
            "score": 0.4,
            "passed": false,
            "findings": [{"severity": "error", "message": "Missing feature."}],
            "failureReport": {
                "contractType": "cadastrophe.failure_report.v1",
                "reason": "missing_feature",
                "nextAction": "outer_loop_refine_source"
            }
        }),
    );

    let fail_output = vlm_submit(
        &args([
            ("session", &failed.session_id),
            ("run", &failed.run_id),
            ("artifact", &failed.artifact_id),
            ("report", fail_report.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(
        fail_output.data["next_action"].as_str(),
        Some("outer_loop_refine_source")
    );
    let failed_state = service.get_session_state(&failed.session_id).unwrap();
    assert_eq!(failed_state.workflow.pending_vlm.len(), 0);
    assert_eq!(failed_state.workflow.outer_iterations.len(), 1);
    assert!(!failed_state.workflow.outer_iterations[0].passed);

    let passed = setup_pending_vlm(&service, &app_data_dir, "pass");
    let pass_report = write_json_file(
        &app_data_dir,
        "vlm-pass.json",
        &json!({
            "contractType": "cadastrophe.vlm_judge_report.v1",
            "runId": passed.run_id,
            "artifactId": passed.artifact_id,
            "score": 0.95,
            "passed": true,
            "findings": []
        }),
    );

    let pass_output = vlm_submit(
        &args([
            ("session", &passed.session_id),
            ("run", &passed.run_id),
            ("artifact", &passed.artifact_id),
            ("report", pass_report.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(pass_output.data["next_action"].as_str(), Some("complete"));
    let passed_state = service.get_session_state(&passed.session_id).unwrap();
    let passed_iterations = passed_state
        .workflow
        .outer_iterations
        .iter()
        .filter(|iteration| iteration.run_id == passed.run_id)
        .collect::<Vec<_>>();
    assert_eq!(passed_iterations.len(), 1);
    assert!(passed_iterations[0].passed);
    let run = passed_state
        .agent_runs
        .iter()
        .find(|run| run.id == passed.run_id)
        .unwrap();
    assert_eq!(run.status, CadAgentRunStatus::Completed);
}

#[derive(Debug)]
struct Setup {
    session_id: String,
    run_id: String,
    revision_id: String,
}

#[derive(Debug)]
struct PendingSetup {
    session_id: String,
    run_id: String,
    artifact_id: String,
}

#[cfg(unix)]
fn setup_pending_vlm(
    service: &SessionService,
    app_data_dir: &PathBuf,
    suffix: &str,
) -> PendingSetup {
    let setup = setup_run_with_plan(service);
    let sidecar = fixture_sidecar(
        app_data_dir,
        &format!("structural-pass-{suffix}"),
        &structural_report_json(&setup.run_id, &setup.revision_id, true),
    );
    let renderer_sidecar =
        fixture_renderer_sidecar(app_data_dir, &format!("renderer-pass-{suffix}"));
    finalize(
        &args([
            ("session", &setup.session_id),
            ("run", &setup.run_id),
            ("revision", &setup.revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
            ("renderer-sidecar", renderer_sidecar.to_str().unwrap()),
        ]),
        service,
        app_data_dir,
    )
    .unwrap();
    let state = service.get_session_state(&setup.session_id).unwrap();
    let pending = state
        .workflow
        .pending_vlm
        .iter()
        .find(|pending| pending.run_id == setup.run_id)
        .unwrap();
    PendingSetup {
        session_id: setup.session_id,
        run_id: setup.run_id,
        artifact_id: pending.artifact_id.clone(),
    }
}

fn setup_run_with_plan(service: &SessionService) -> Setup {
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let input_revision_id = Some(create_test_revision(service, &created.session_id));
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Create a wall bracket.".to_string(),
            input_revision_id.clone(),
            Some("test".to_string()),
            None,
        )
        .unwrap();
    let plan: CadModelPlan = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/cad_model_plan.v1.json"
    ))
    .unwrap();
    service
        .save_workflow_plan(
            &created.session_id,
            CadWorkflowPlan {
                run_id: run.id.clone(),
                revision_id: input_revision_id.clone(),
                plan: plan.clone(),
                source_language: plan.source_language.clone(),
                created_at: timestamp(),
            },
        )
        .unwrap();
    let source_result = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "// @main_component wall_bracket\ncube([3, 1, 2]);".to_string(),
            parent_revision_id: input_revision_id,
            parameters: None,
        })
        .unwrap();
    service
        .link_agent_run_output_revision(
            &created.session_id,
            &run.id,
            source_result.revision_id.clone(),
        )
        .unwrap();
    Setup {
        session_id: created.session_id,
        run_id: run.id,
        revision_id: source_result.revision_id,
    }
}

fn create_test_revision(service: &SessionService, session_id: &str) -> String {
    service
        .update_model_source(UpdateModelSourceInput {
            session_id: session_id.to_string(),
            source_language: CadSourceLanguage::Openscad,
            source: "cube([2, 2, 2]);".to_string(),
            parent_revision_id: None,
            parameters: None,
        })
        .unwrap()
        .revision_id
}

fn sqlite_service(app_data_dir: &PathBuf) -> SessionService {
    let layout = StorageLayout::from_app_data_dir(app_data_dir.clone());
    storage::initialize_storage(&layout).unwrap();
    SessionService::with_repository_without_startup_verification(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap()
}

#[cfg(unix)]
fn fixture_sidecar(app_data_dir: &PathBuf, name: &str, report: &Value) -> PathBuf {
    let path = app_data_dir.join(name);
    fs::create_dir_all(app_data_dir).unwrap();
    fs::write(
        &path,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{}'\n",
            serde_json::to_string(report)
                .unwrap()
                .replace('\'', "'\\''")
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
fn fixture_renderer_sidecar(app_data_dir: &PathBuf, name: &str) -> PathBuf {
    let path = app_data_dir.join(name);
    let png_path = app_data_dir.join(format!("{name}.png"));
    const PNG_BYTES: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8,
        0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe, 0xdc, 0xcc, 0x59, 0xe7, 0x00, 0x00, 0x00,
        0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
    ];
    fs::create_dir_all(app_data_dir).unwrap();
    fs::write(&png_path, PNG_BYTES).unwrap();
    let sha256 = storage::sha256_hex(PNG_BYTES);
    let png_path = png_path.to_string_lossy().replace('\\', "\\\\");
    fs::write(
            &path,
            format!(
                r#"#!/bin/sh
input=$(cat)
artifact_id=$(printf '%s' "$input" | sed -n 's/.*"artifactId":"\([^"]*\)".*/\1/p')
revision_id=$(printf '%s' "$input" | sed -n 's/.*"revisionId":"\([^"]*\)".*/\1/p')
run_id=$(printf '%s' "$input" | sed -n 's/.*"runId":"\([^"]*\)".*/\1/p')
source_sha=$(printf '%s' "$input" | sed -n 's/.*"sourceArtifactSha256":"\([^"]*\)".*/\1/p')
source_hash=$(printf '%s' "$input" | sed -n 's/.*"sourceHash":"\([^"]*\)".*/\1/p')
cat <<EOF
{{"contractType":"cadastrophe.vlm_render_manifest.v1","runId":"$run_id","revisionId":"$revision_id","artifactId":"$artifact_id","sourceArtifactId":"$artifact_id","sourceArtifactSha256":"$source_sha","sourceHash":"$source_hash","format":"png","path":"{}","sha256":"{}","bytes":{}.0,"renderer":"cadastrophe-vlm-renderer","rendererEngine":"fixture-renderer","viewMode":"9-view","resolution":{{"width":1,"height":1}},"views":["Front-Left-Top","Front","Front-Right-Top","Left","Top","Right","Bottom","Back","Back-Right-Top"]}}
EOF
"#,
                png_path,
                sha256,
                PNG_BYTES.len()
            ),
        )
        .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn structural_report_json(run_id: &str, revision_id: &str, passed: bool) -> Value {
    let mut report = json!({
        "contractType": "cadastrophe.structural_report.v1",
        "runId": run_id,
        "revisionId": revision_id,
        "artifactId": "artifact-from-fixture-sidecar",
        "passed": passed,
        "checks": [
            {
                "name": "fixture_structural_anchor",
                "passed": passed,
                "severity": if passed { "info" } else { "error" },
                "message": if passed { "Structural fixture passed." } else { "Structural fixture failed." }
            }
        ]
    });
    if !passed {
        report["failureReport"] = json!({
            "contractType": "cadastrophe.failure_report.v1",
            "reason": "fixture_structural_anchor_failed",
            "nextAction": "refine_plan_or_source"
        });
    }
    report
}

fn write_json_file(app_data_dir: &PathBuf, name: &str, value: &Value) -> PathBuf {
    let path = app_data_dir.join(name);
    fs::write(&path, serde_json::to_string(value).unwrap()).unwrap();
    path
}

fn args<const N: usize>(values: [(&str, &str); N]) -> ParsedArgs {
    ParsedArgs {
        pretty: false,
        values: values
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
    }
}

fn temp_app_data_dir(name: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    std::env::temp_dir().join(format!(
        "cadastrophe-cli-{name}-{}-{millis}",
        std::process::id()
    ))
}
