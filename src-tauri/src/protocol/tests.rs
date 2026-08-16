use super::*;

#[test]
fn cad_model_plan_fixture_matches_rust_schema() {
    let plan: CadModelPlan = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/cad_model_plan.v1.json"
    ))
    .unwrap();

    assert_eq!(plan.schema_version, "cad_model_plan.v1");
    assert_eq!(plan.main_component.name, "wall_bracket");
    assert_eq!(plan.source_language, CadSourceLanguage::Openscad);
    assert_eq!(
        plan.runtime_constraints.runtime,
        CadRuntimeKind::OpenscadWasm
    );
    assert!(plan.expected_aspect_ratio.tolerance > 0.0);
}

#[test]
fn workflow_state_fixture_matches_rust_schema() {
    let workflow: CadWorkflowState = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/workflow_state.v1.json"
    ))
    .unwrap();

    assert_eq!(workflow.plans.len(), 1);
    assert_eq!(workflow.outer_iterations.len(), 1);
    assert_eq!(workflow.pending_vlm.len(), 1);
    assert_eq!(workflow.plans[0].plan.schema_version, "cad_model_plan.v1");
    assert_eq!(
        workflow.pending_vlm[0]
            .dfm_report
            .as_ref()
            .and_then(|report| report.get("contractType"))
            .and_then(Value::as_str),
        Some("cadastrophe.dfm_report.v1")
    );
    let serialized = serde_json::to_value(&workflow).unwrap();
    assert_eq!(
        serialized["pendingVlm"][0]["dfmReport"]["profileHash"].as_str(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(
        workflow.outer_iterations[0]
            .failure_report
            .as_ref()
            .and_then(|report| report.get("contractType"))
            .and_then(Value::as_str),
        Some("cadastrophe.failure_report.v1")
    );
    assert_eq!(
        workflow.pending_vlm[0]
            .contract
            .get("contractType")
            .and_then(Value::as_str),
        Some("cadastrophe.vlm_judge.v1")
    );
}

#[test]
fn report_contract_fixtures_keep_expected_discriminators() {
    for (fixture, contract_type) in [
        (
            include_str!("../../../fixtures/contracts/structural_report.v1.json"),
            "cadastrophe.structural_report.v1",
        ),
        (
            include_str!("../../../fixtures/contracts/dfm_report.v1.json"),
            "cadastrophe.dfm_report.v1",
        ),
        (
            include_str!("../../../fixtures/contracts/vlm_judge_contract.v1.json"),
            "cadastrophe.vlm_judge.v1",
        ),
        (
            include_str!("../../../fixtures/contracts/vlm_submission.v1.json"),
            "cadastrophe.vlm_submission.v1",
        ),
        (
            include_str!("../../../fixtures/contracts/vlm_judge_report.v1.json"),
            "cadastrophe.vlm_judge_report.v1",
        ),
    ] {
        let value: Value = serde_json::from_str(fixture).unwrap();
        assert_eq!(
            value.get("contractType").and_then(Value::as_str),
            Some(contract_type)
        );
    }
}
