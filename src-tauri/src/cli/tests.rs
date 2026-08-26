use super::model_commands::{plan_commit, source_apply};
use super::session_commands::session_state;
use super::workflow_commands::finalize;
use super::*;
use crate::protocol::{CadModelPlan, CadValidationBatchStatus, CreateCadSessionInput};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn plan_commit_honors_explicit_run_revision_after_ui_active_revision_changes() {
    let app_data_dir = temp_app_data_dir("explicit-plan-revision-scope");
    let service = sqlite_service(&app_data_dir);
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let input_revision_id = create_test_revision(&service, &created.session_id);
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "scoped plan".to_string(),
            Some(input_revision_id.clone()),
            Some("test".to_string()),
            None,
        )
        .unwrap();
    let _unrelated_active = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "sphere(r = 2);".to_string(),
            parent_revision_id: Some(input_revision_id.clone()),
            parameters: None,
        })
        .unwrap();
    let plan_path = write_json_file(&app_data_dir, "scoped-plan.json", &draft_plan_value());

    let output = plan_commit(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("revision", &input_revision_id),
            ("plan", plan_path.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();
    assert_eq!(output.data["revisionId"], input_revision_id);

    let error = plan_commit(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("plan", plan_path.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap_err();
    assert_eq!(error.code, "precondition_failed");
    assert!(error.message.contains("not an input or output revision"));
}

#[test]
fn session_state_rejects_run_from_another_session() {
    let app_data_dir = temp_app_data_dir("session-state-run-scope");
    let service = sqlite_service(&app_data_dir);
    let first = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let second = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &first.session_id,
            "first session".to_string(),
            None,
            Some("test".to_string()),
            None,
        )
        .unwrap();
    let error = session_state(
        &args([("session", &second.session_id), ("run", &run.id)]),
        &service,
        &app_data_dir,
    )
    .unwrap_err();
    assert_eq!(error.code, "not_found");
}

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
    assert!(error.message.contains("cadgen-ax-plan-commit"));
}

#[cfg(unix)]
#[test]
fn finalize_structural_fail_returns_report_without_prusaslicer() {
    let app_data_dir = temp_app_data_dir("finalize-structural-fail");
    let service = sqlite_service(&app_data_dir);
    let setup = setup_run_with_plan(&service);
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-fail",
        &structural_report_json(&setup.run_id, &setup.revision_id, false),
    );
    let renderer_sidecar = fixture_renderer_sidecar(&app_data_dir, "renderer-structural-fail");
    let prusaslicer = fixture_prusaslicer(&app_data_dir, "prusaslicer-structural-fail", None);
    let profile = fixture_dfm_profile(&app_data_dir);
    let output = finalize(
        &args([
            ("session", &setup.session_id),
            ("run", &setup.run_id),
            ("revision", &setup.revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
            ("renderer-sidecar", renderer_sidecar.to_str().unwrap()),
            ("prusaslicer-path", prusaslicer.to_str().unwrap()),
            ("dfm-profile", profile.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(
        output.data["next_action"].as_str(),
        Some("validation_queued")
    );
    let state = service.get_session_state(&setup.session_id).unwrap();
    assert_eq!(state.workflow.pending_vlm.len(), 0);
    assert!(state.workflow.outer_iterations.is_empty());
    assert_eq!(state.validation_batches.len(), 1);
    assert_eq!(state.validation_checks.len(), 3);
    let completed = state
        .agent_run_events
        .iter()
        .rev()
        .find(|event| {
            event.event_type == CadAgentRunEventType::AgentToolCompleted
                && event.payload["command"] == "cadgen-ax-finalize"
        })
        .expect("finalize completion event");
    assert_eq!(
        completed.payload["nextAction"].as_str(),
        Some("validation_queued")
    );
    let reloaded_state = sqlite_service(&app_data_dir)
        .get_session_state(&setup.session_id)
        .unwrap();
    assert_eq!(reloaded_state.validation_batches.len(), 1);
    assert_eq!(reloaded_state.validation_checks.len(), 3);
}

#[cfg(unix)]
#[test]
fn finalize_structural_pass_queues_validation_evaluation() {
    let app_data_dir = temp_app_data_dir("finalize-structural-pass");
    let service = sqlite_service(&app_data_dir);
    let setup = setup_run_with_plan(&service);
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-pass",
        &structural_report_json(&setup.run_id, &setup.revision_id, true),
    );
    let renderer_sidecar = fixture_renderer_sidecar(&app_data_dir, "renderer-pass");
    let prusaslicer = fixture_prusaslicer(&app_data_dir, "prusaslicer-pass", None);
    let profile = fixture_dfm_profile(&app_data_dir);

    let output = finalize(
        &args([
            ("session", &setup.session_id),
            ("run", &setup.run_id),
            ("revision", &setup.revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
            ("renderer-sidecar", renderer_sidecar.to_str().unwrap()),
            ("prusaslicer-path", prusaslicer.to_str().unwrap()),
            ("dfm-profile", profile.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(
        output.data["next_action"].as_str(),
        Some("validation_queued")
    );
    let batch_id = output.data["validationBatch"]["id"].as_str().unwrap();
    let state = service.get_session_state(&setup.session_id).unwrap();
    let queued = state
        .validation_batches
        .iter()
        .find(|batch| batch.id == batch_id)
        .unwrap();
    assert_eq!(queued.status, CadValidationBatchStatus::Queued);
    assert_eq!(queued.run_id, setup.run_id);
    assert_eq!(queued.revision_id, setup.revision_id);
    assert!(state.workflow.pending_vlm.is_empty());
    assert_eq!(state.workflow.outer_iterations.len(), 0);
    assert_eq!(state.validation_checks.len(), 3);
    assert!(state.validation_checks.iter().all(|check| {
        check.input_contract["batchId"] == batch_id
            && check.input_contract["checkId"] == check.id
            && check.input_contract["evaluationId"] == check.id
    }));
}

#[cfg(unix)]
#[test]
fn finalize_dfm_fail_returns_both_reports_without_pending_vlm() {
    let app_data_dir = temp_app_data_dir("finalize-dfm-fail");
    let service = sqlite_service(&app_data_dir);
    let setup = setup_run_with_plan(&service);
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-pass-dfm-fail",
        &structural_report_json(&setup.run_id, &setup.revision_id, true),
    );
    let prusaslicer = fixture_prusaslicer(
        &app_data_dir,
        "prusaslicer-dfm-fail",
        Some("ERROR: model has an unprintable feature"),
    );
    let profile = fixture_dfm_profile(&app_data_dir);
    let renderer_sidecar = fixture_renderer_sidecar(&app_data_dir, "renderer-dfm-fail");

    let output = finalize(
        &args([
            ("session", &setup.session_id),
            ("run", &setup.run_id),
            ("revision", &setup.revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
            ("prusaslicer-path", prusaslicer.to_str().unwrap()),
            ("dfm-profile", profile.to_str().unwrap()),
            ("renderer-sidecar", renderer_sidecar.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(output.data["nextAction"], "validation_queued");
    let state = service.get_session_state(&setup.session_id).unwrap();
    assert!(state.workflow.pending_vlm.is_empty());
    assert!(state.workflow.outer_iterations.is_empty());
    assert_eq!(state.validation_checks.len(), 3);
}

#[cfg(unix)]
#[test]
fn finalize_fails_fast_when_prusaslicer_produces_no_gcode() {
    let app_data_dir = temp_app_data_dir("finalize-no-gcode");
    let service = sqlite_service(&app_data_dir);
    let setup = setup_run_with_plan(&service);
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-pass-no-gcode",
        &structural_report_json(&setup.run_id, &setup.revision_id, true),
    );
    let prusaslicer = fixture_prusaslicer_without_gcode(&app_data_dir, "prusaslicer-no-gcode");
    let profile = fixture_dfm_profile(&app_data_dir);
    let renderer_sidecar = fixture_renderer_sidecar(&app_data_dir, "renderer-no-gcode");

    let output = finalize(
        &args([
            ("session", &setup.session_id),
            ("run", &setup.run_id),
            ("revision", &setup.revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
            ("prusaslicer-path", prusaslicer.to_str().unwrap()),
            ("dfm-profile", profile.to_str().unwrap()),
            ("renderer-sidecar", renderer_sidecar.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(output.data["nextAction"], "validation_queued");
    assert!(service
        .get_session_state(&setup.session_id)
        .unwrap()
        .workflow
        .pending_vlm
        .is_empty());
}

#[cfg(unix)]
#[test]
fn finalize_fails_fast_when_prusaslicer_is_not_configured() {
    let app_data_dir = temp_app_data_dir("finalize-prusaslicer-unset");
    let service = sqlite_service(&app_data_dir);
    let setup = setup_run_with_plan(&service);
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-pass-prusaslicer-unset",
        &structural_report_json(&setup.run_id, &setup.revision_id, true),
    );

    let error = finalize(
        &args([
            ("session", &setup.session_id),
            ("run", &setup.run_id),
            ("revision", &setup.revision_id),
            ("sidecar", sidecar.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap_err();

    assert_eq!(error.code, "invalid_input");
    assert!(error.message.contains("renderer sidecar"));
}

#[cfg(unix)]
#[test]
fn dfm_executable_validation_rejects_missing_and_broken_binary() {
    let app_data_dir = temp_app_data_dir("dfm-binary-validation");
    fs::create_dir_all(&app_data_dir).unwrap();
    let missing = app_data_dir.join("missing-prusaslicer");
    assert!(crate::dfm::validate_executable(missing.to_str().unwrap())
        .unwrap_err()
        .contains("does not exist"));

    let broken = app_data_dir.join("broken-prusaslicer");
    fs::write(&broken, "#!/bin/sh\nprintf '%s\\n' 'broken' >&2\nexit 9\n").unwrap();
    let mut permissions = fs::metadata(&broken).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&broken, permissions).unwrap();
    let error = crate::dfm::validate_executable(broken.to_str().unwrap()).unwrap_err();
    assert!(error.contains("exited with status"));
    assert!(error.contains("broken"));
}

#[cfg(unix)]
#[test]
fn dfm_executable_validation_uses_supported_help_action() {
    let app_data_dir = temp_app_data_dir("dfm-binary-help-validation");
    fs::create_dir_all(&app_data_dir).unwrap();
    let prusaslicer = app_data_dir.join("PrusaSlicer");
    fs::write(
        &prusaslicer,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '%s\\n' 'PrusaSlicer-2.9.6 based on Slic3r (with GUI support)'\n  printf '%s\\n' 'Unknown option --version' >&2\n  exit 1\nfi\nif [ \"$1\" = \"--help\" ]; then\n  printf '%s\\n' 'PrusaSlicer-2.9.6 based on Slic3r (with GUI support)'\n  exit 0\nfi\nexit 8\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&prusaslicer).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&prusaslicer, permissions).unwrap();

    let validation = crate::dfm::validate_executable(prusaslicer.to_str().unwrap()).unwrap();
    assert_eq!(
        validation.version,
        "PrusaSlicer-2.9.6 based on Slic3r (with GUI support)"
    );
}

#[cfg(unix)]
#[test]
fn dfm_settings_survive_reload_from_app_data_directory() {
    let app_data_dir = temp_app_data_dir("dfm-settings-restart");
    fs::create_dir_all(&app_data_dir).unwrap();
    let prusaslicer = fixture_prusaslicer(&app_data_dir, "prusaslicer-settings", None);
    let contents = include_str!("../../../profile.ini");
    let executable =
        crate::dfm::save_executable(&app_data_dir, prusaslicer.to_str().unwrap()).unwrap();
    let profile = crate::dfm::save_profile(&app_data_dir, contents).unwrap();

    // get_settings reconstructs state exclusively from persisted app-data files,
    // matching what a newly started backend process reads.
    let reloaded = crate::dfm::get_settings(&app_data_dir).unwrap();
    assert_eq!(
        reloaded.prusaslicer_executable.as_deref(),
        Some(executable.path.as_str())
    );
    assert_eq!(reloaded.executable_validation, Some(executable));
    assert_eq!(reloaded.profile.hash, profile.hash);
    assert_eq!(reloaded.profile.contents, contents);
}

#[test]
fn dfm_profile_validation_rejects_invalid_profile() {
    let error =
        crate::dfm::validate_profile("printer_technology = FFF\nnozzle_diameter = not-a-number\n")
            .unwrap_err();
    assert!(error.contains("filament_diameter") || error.contains("numeric"));
}

#[cfg(unix)]
#[test]
fn cli_workflow_persists_required_tool_event_order() {
    let app_data_dir = temp_app_data_dir("workflow-event-order");
    let service = sqlite_service(&app_data_dir);
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    service.mark_session_viewed(&created.session_id).unwrap();
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
    let plan_path = write_json_file(&app_data_dir, "plan.json", &draft_plan_value());
    let source_path = app_data_dir.join("source.scad");
    fs::write(
        &source_path,
        "// @main_component wall_bracket\ncube([3, 1, 2]);\n",
    )
    .unwrap();

    plan_commit(
        &args([("plan", plan_path.to_str().unwrap())]),
        &service,
        &app_data_dir,
    )
    .unwrap();
    let source_output = source_apply(
        &args([("source", source_path.to_str().unwrap())]),
        &service,
        &app_data_dir,
    )
    .unwrap();
    let revision_id = source_output.data["revisionId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(source_output.data["nextAction"].as_str(), Some("finalize"));
    let sidecar = fixture_sidecar(
        &app_data_dir,
        "structural-event-order-pass",
        &structural_report_json(&run.id, &revision_id, true),
    );
    let renderer_sidecar = fixture_renderer_sidecar(&app_data_dir, "renderer-event-order");
    let prusaslicer = fixture_prusaslicer(&app_data_dir, "prusaslicer-event-order", None);
    let profile = fixture_dfm_profile(&app_data_dir);
    finalize(
        &args([
            ("sidecar", sidecar.to_str().unwrap()),
            ("renderer-sidecar", renderer_sidecar.to_str().unwrap()),
            ("prusaslicer-path", prusaslicer.to_str().unwrap()),
            ("dfm-profile", profile.to_str().unwrap()),
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
            "cadgen-ax-plan-commit",
            "cadgen-ax-source-apply",
            "cadgen-ax-finalize",
        ]
    );
    assert_eq!(state.workflow.plans.len(), 1);
    assert_eq!(state.validation_batches.len(), 1);
    assert_eq!(state.validation_checks.len(), 3);
    assert!(state.workflow.pending_vlm.is_empty());
}

#[test]
fn plan_commit_normalizes_draft_contract_to_full_workflow_plan() {
    let app_data_dir = temp_app_data_dir("plan-draft-normalization");
    let service = sqlite_service(&app_data_dir);
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Create a wall bracket.".to_string(),
            created.state.session.active_revision_id.clone(),
            Some("test".to_string()),
            None,
        )
        .unwrap();
    let plan_path = write_json_file(&app_data_dir, "draft-plan.json", &draft_plan_value());

    let output = plan_commit(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("plan", plan_path.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap();

    assert_eq!(
        output.data["plan"]["schemaVersion"].as_str(),
        Some("cad_model_plan.v1")
    );
    assert_eq!(
        output.data["plan"]["sourceLanguage"].as_str(),
        Some("openscad")
    );
    assert_eq!(
        output.data["plan"]["runtimeConstraints"]["runtime"].as_str(),
        Some("openscad-wasm")
    );
    assert_eq!(
        output.data["plan"]["runtimeConstraints"]["mainComponentAnnotation"].as_str(),
        Some("// @main_component wall_bracket")
    );
    assert_eq!(
        output.data["plan"]["expectedAspectRatio"]["tolerance"].as_f64(),
        Some(0.25)
    );
    assert!(
        output.data["plan"]["runtimeConstraints"]["forbiddenFeatures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|feature| feature.as_str() == Some("external_file_include"))
    );
    assert!(
        output.data["plan"]["runtimeConstraints"]["requiredFeatures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|feature| feature.as_str() == Some("two_aligned_holes"))
    );

    let state = service.get_session_state(&created.session_id).unwrap();
    assert_eq!(state.workflow.plans.len(), 1);
    assert_eq!(
        state.workflow.plans[0]
            .plan
            .runtime_constraints
            .main_component_annotation
            .as_deref(),
        Some("// @main_component wall_bracket")
    );
}

#[test]
fn plan_commit_rejects_agent_authored_runtime_policy() {
    let app_data_dir = temp_app_data_dir("plan-runtime-policy-rejection");
    let service = sqlite_service(&app_data_dir);
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Create a cadquery model.".to_string(),
            created.state.session.active_revision_id.clone(),
            Some("test".to_string()),
            None,
        )
        .unwrap();
    let full_plan = json!({
        "schemaVersion": "cad_model_plan.v1",
        "summary": "Unsupported runtime policy attempt.",
        "mainComponent": {
            "name": "wall_bracket",
            "purpose": "single printable bracket body"
        },
        "supportingComponents": [],
        "expectedAspectRatio": {
            "x": 3.0,
            "y": 1.0,
            "z": 2.0,
            "tolerance": 0.25
        },
        "sourceLanguage": "cadquery",
        "runtimeConstraints": {
            "runtime": "cadquery-local",
            "forbiddenFeatures": []
        }
    });
    let plan_path = write_json_file(&app_data_dir, "full-plan.json", &full_plan);

    let error = plan_commit(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("plan", plan_path.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap_err();

    assert_eq!(error.code, "invalid_input");
    assert!(error.message.contains("system-owned runtime policy"));
    assert!(error.message.contains("runtimeConstraints"));
    let state = service.get_session_state(&created.session_id).unwrap();
    assert!(state.workflow.plans.is_empty());
}

#[test]
fn plan_commit_rejects_agent_authored_aspect_ratio_tolerance() {
    let app_data_dir = temp_app_data_dir("plan-tolerance-rejection");
    let service = sqlite_service(&app_data_dir);
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Create a wall bracket.".to_string(),
            created.state.session.active_revision_id.clone(),
            Some("test".to_string()),
            None,
        )
        .unwrap();
    let mut draft_plan = draft_plan_value();
    draft_plan["expectedAspectRatio"]["tolerance"] = json!(0.25);
    let plan_path = write_json_file(&app_data_dir, "draft-plan-with-tolerance.json", &draft_plan);

    let error = plan_commit(
        &args([
            ("session", &created.session_id),
            ("run", &run.id),
            ("plan", plan_path.to_str().unwrap()),
        ]),
        &service,
        &app_data_dir,
    )
    .unwrap_err();

    assert_eq!(error.code, "invalid_input");
    assert!(error.message.contains("tolerance"));
    let state = service.get_session_state(&created.session_id).unwrap();
    assert!(state.workflow.plans.is_empty());
}

#[test]
fn source_apply_runtime_failure_records_source_repair_event_diagnostics() {
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
    let plan_path = write_json_file(&app_data_dir, "repair-plan.json", &draft_plan_value());
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

    assert_eq!(source_output.data["next_action"].as_str(), None);
    assert_eq!(
        source_output.data["nextAction"].as_str(),
        Some("source_repair")
    );
    let state = service.get_session_state(&created.session_id).unwrap();
    let source_apply_event = state
        .agent_run_events
        .iter()
        .rev()
        .find(|event| {
            event.run_id == run.id
                && event.event_type == CadAgentRunEventType::AgentToolCompleted
                && event.payload.get("command").and_then(Value::as_str)
                    == Some("cadgen-ax-source-apply")
        })
        .unwrap();
    assert_eq!(
        source_apply_event
            .payload
            .get("nextAction")
            .and_then(Value::as_str),
        Some("source_repair")
    );
    assert!(source_apply_event
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

#[derive(Debug)]
struct Setup {
    session_id: String,
    run_id: String,
    revision_id: String,
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
{{"contractType":"cadgen-ax.vlm_render_manifest.v1","runId":"$run_id","revisionId":"$revision_id","artifactId":"$artifact_id","sourceArtifactId":"$artifact_id","sourceArtifactSha256":"$source_sha","sourceHash":"$source_hash","format":"png","path":"{}","sha256":"{}","bytes":{}.0,"renderer":"cadgen-ax-vlm-renderer","rendererEngine":"fixture-renderer","viewMode":"9-view","resolution":{{"width":1,"height":1}},"views":["Front-Left-Top","Front","Front-Right-Top","Left","Top","Right","Bottom","Back","Back-Right-Top"]}}
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

#[cfg(unix)]
fn fixture_prusaslicer(app_data_dir: &PathBuf, name: &str, diagnostic: Option<&str>) -> PathBuf {
    let path = app_data_dir.join(name);
    let diagnostic = diagnostic.unwrap_or("").replace('\'', "'\\''");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--help\" ]; then\n  printf '%s\\n' 'PrusaSlicer 2.9.6'\n  exit 0\nfi\nif [ \"$#\" -ne 4 ] || [ \"$1\" != \"--load\" ] || [ \"$3\" != \"--export-gcode\" ]; then\n  printf '%s\\n' 'unexpected arguments' >&2\n  exit 8\nfi\nlast=''\nfor arg in \"$@\"; do last=\"$arg\"; done\nprintf '%s\\n' '; generated fixture G-code' > \"${{last%.stl}}.gcode\"\nprintf '%s\\n' '{}'\n",
            diagnostic
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
fn fixture_prusaslicer_without_gcode(app_data_dir: &PathBuf, name: &str) -> PathBuf {
    let path = app_data_dir.join(name);
    fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = \"--help\" ]; then\n  printf '%s\\n' 'PrusaSlicer 2.9.6'\nfi\nexit 0\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn fixture_dfm_profile(app_data_dir: &PathBuf) -> PathBuf {
    let path = app_data_dir.join("dfm-profile.ini");
    fs::write(&path, include_str!("../../../profile.ini")).unwrap();
    path
}

fn structural_report_json(run_id: &str, revision_id: &str, passed: bool) -> Value {
    let mut report = json!({
        "contractType": "cadgen-ax.structural_report.v1",
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
            "contractType": "cadgen-ax.failure_report.v1",
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

#[test]
fn vlm_submit_builds_only_validated_score_submission_data() {
    let submission = vlm::build_vlm_submission(&args([
        ("components", "3"),
        ("proportions", "2"),
        ("structure", "3"),
        ("inconsistency", "rear view differs slightly"),
        ("diagnostic", "all requested components are visible"),
    ]))
    .unwrap();
    assert_eq!(
        submission,
        json!({
            "contractType": "cadgen-ax.vlm_submission.v1",
            "scores": {"structure": 3, "components": 3, "proportions": 2},
            "inconsistencies": ["rear view differs slightly"],
            "diagnostic": "all requested components are visible"
        })
    );

    assert!(vlm::build_vlm_submission(&args([
        ("components", "4"),
        ("proportions", "2"),
        ("structure", "3"),
    ]))
    .unwrap_err()
    .message
    .contains("0 through 3"));
    assert!(vlm::build_vlm_submission(&args([
        ("components", "3"),
        ("proportions", "2"),
        ("structure", "3"),
        ("passed", "true"),
    ]))
    .unwrap_err()
    .message
    .contains("Unsupported --passed"));
}

#[test]
fn cli_argument_parser_rejects_duplicate_options() {
    let error = parse_args(
        ["--structure", "3", "--structure", "2"]
            .into_iter()
            .map(str::to_string),
    )
    .unwrap_err();
    assert!(error.message.contains("Duplicate --structure"));
}

fn draft_plan_value() -> Value {
    json!({
        "summary": "Parametric wall bracket with a back plate, two screw holes, and a forward support tab.",
        "mainComponent": {
            "name": "wall_bracket",
            "purpose": "single printable bracket body",
            "requiredFeatures": ["back_plate", "screw_holes", "support_tab"]
        },
        "supportingComponents": [
            {
                "name": "screw_holes",
                "purpose": "mounting clearance holes through the back plate",
                "requiredFeatures": ["two_aligned_holes"]
            },
            {
                "name": "support_tab",
                "purpose": "horizontal tab for carrying a small load",
                "requiredFeatures": ["rounded_outer_edge"]
            }
        ],
        "expectedAspectRatio": {
            "x": 3.0,
            "y": 1.0,
            "z": 2.0
        }
    })
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
        "cadgen-ax-cli-{name}-{}-{millis}",
        std::process::id()
    ))
}
