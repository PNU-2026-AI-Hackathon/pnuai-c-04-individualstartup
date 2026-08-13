use crate::protocol::{
    CadArtifact, CadArtifactKind, CadValidationEvaluation, CadValidationEvaluationKind,
};
use crate::storage;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const INPUT_CONTRACT_TYPE: &str = "cadastrophe.vlm_evaluation_input.v1";
pub const REPORT_CONTRACT_TYPE: &str = "cadastrophe.vlm_judge_report.v1";
pub const FAILURE_REPORT_CONTRACT_TYPE: &str = "cadastrophe.failure_report.v1";

#[derive(Clone, Debug)]
pub struct EvaluationContractInput<'a> {
    pub session_id: &'a str,
    pub run_id: &'a str,
    pub revision_id: &'a str,
    pub user_request: &'a str,
    pub final_artifact: &'a CadArtifact,
    pub rendered_image: &'a CadArtifact,
    pub pass_threshold: f64,
    pub judge_contract: &'a Value,
    pub structural_report: &'a Value,
    pub dfm_report: &'a Value,
    pub app_data_dir: &'a Path,
}

#[derive(Clone, Debug)]
pub struct ValidatedReport {
    pub report: Value,
    pub score: f64,
    pub passed: bool,
    pub failure_report: Option<Value>,
}

pub fn build_input_contract(input: EvaluationContractInput<'_>) -> Result<Value, String> {
    require_id("sessionId", input.session_id)?;
    require_id("runId", input.run_id)?;
    require_id("revisionId", input.revision_id)?;
    if input.user_request.trim().is_empty() {
        return Err("VLM evaluation userRequest cannot be empty.".to_string());
    }
    if !input.pass_threshold.is_finite() || !(0.0..=1.0).contains(&input.pass_threshold) {
        return Err("VLM evaluation passThreshold must be finite and between 0 and 1.".to_string());
    }
    validate_artifact_lineage(
        input.final_artifact,
        input.revision_id,
        CadArtifactKind::Stl,
        "stl",
    )?;
    validate_artifact_lineage(
        input.rendered_image,
        input.revision_id,
        CadArtifactKind::RenderImage,
        "png",
    )?;
    let final_file = verified_artifact_file(input.app_data_dir, input.final_artifact)?;
    let rendered_file = verified_artifact_file(input.app_data_dir, input.rendered_image)?;
    let rendered_source = metadata_string(input.rendered_image, "sourceArtifactId")?;
    if rendered_source != input.final_artifact.id {
        return Err(format!(
            "Rendered image {} references STL {}, not {}.",
            input.rendered_image.id, rendered_source, input.final_artifact.id
        ));
    }
    let judge = input
        .judge_contract
        .as_object()
        .ok_or_else(|| "VLM judge contract must be a JSON object.".to_string())?;
    if judge.get("contractType").and_then(Value::as_str) != Some("cadastrophe.vlm_judge.v1") {
        return Err("VLM judge contractType must be cadastrophe.vlm_judge.v1.".to_string());
    }
    require_object_contract(input.structural_report, "structural report")?;
    require_object_contract(input.dfm_report, "DFM report")?;

    Ok(json!({
        "contractType": INPUT_CONTRACT_TYPE,
        "sessionId": input.session_id,
        "runId": input.run_id,
        "revisionId": input.revision_id,
        "artifactId": input.final_artifact.id,
        "kind": "vlm",
        "userRequest": input.user_request,
        "passThreshold": input.pass_threshold,
        "finalArtifact": {
            "artifactId": input.final_artifact.id,
            "kind": "stl",
            "path": final_file.path,
            "sha256": final_file.sha256,
        },
        "renderedImage": {
            "artifactId": input.rendered_image.id,
            "path": rendered_file.path,
            "sha256": rendered_file.sha256,
            "mediaType": "image/png",
        },
        "judgeContract": input.judge_contract,
        "structuralReport": input.structural_report,
        "dfmReport": input.dfm_report,
        "rubric": {
            "subscoreRange": { "minimum": 0, "maximum": 3 },
            "composite": "scores.structure + scores.components + scores.proportions",
            "score": "composite / 9",
            "majorMissingFeatureFails": true,
            "passRequiresThreshold": true
        },
        "outputContract": {
            "contractType": REPORT_CONTRACT_TYPE,
            "identityFields": ["evaluationId", "sessionId", "runId", "revisionId", "artifactId", "kind", "attempt"],
            "requiredArrays": ["findings", "enumeration", "inconsistencies"],
            "failureReportContractType": FAILURE_REPORT_CONTRACT_TYPE,
            "failureReportRequiredFields": ["contractType", "reason", "summary", "nextAction"],
            "failureNextAction": "outer_loop_refine_source"
        }
    }))
}

pub fn rendered_image_path(evaluation: &CadValidationEvaluation) -> Result<PathBuf, String> {
    let rendered = contract_object(&evaluation.input_contract, "renderedImage")?;
    let path = required_string(rendered, "path", "renderedImage")?;
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("VLM renderedImage.path must be absolute.".to_string());
    }
    verify_contract_file(rendered, &path, "renderedImage")?;
    Ok(path)
}

pub fn validate_report(
    evaluation: &CadValidationEvaluation,
    report: Value,
) -> Result<ValidatedReport, String> {
    let object = report
        .as_object()
        .ok_or_else(|| "VLM report must be a JSON object.".to_string())?;
    require_exact_string(object, "contractType", REPORT_CONTRACT_TYPE)?;
    require_exact_string(object, "evaluationId", &evaluation.id)?;
    require_exact_string(object, "sessionId", &evaluation.session_id)?;
    require_exact_string(object, "runId", &evaluation.run_id)?;
    require_exact_string(object, "revisionId", &evaluation.revision_id)?;
    require_exact_string(object, "artifactId", &evaluation.artifact_id)?;
    require_exact_string(object, "kind", "vlm")?;
    let attempt = object
        .get("attempt")
        .and_then(Value::as_u64)
        .ok_or_else(|| "VLM report attempt must be an unsigned integer.".to_string())?;
    if attempt != u64::from(evaluation.attempt) {
        return Err(format!(
            "VLM report attempt mismatch: expected {}, received {attempt}.",
            evaluation.attempt
        ));
    }
    for field in ["findings", "enumeration", "inconsistencies"] {
        object
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("VLM report {field} must be an array."))?;
    }
    validate_findings(object)?;
    validate_enumeration(object)?;
    let scores = object
        .get("scores")
        .and_then(Value::as_object)
        .ok_or_else(|| "VLM report scores must be an object.".to_string())?;
    let structure = subscore(scores, "structure")?;
    let components = subscore(scores, "components")?;
    let proportions = subscore(scores, "proportions")?;
    let expected_composite = structure + components + proportions;
    let composite = object
        .get("composite")
        .and_then(Value::as_u64)
        .ok_or_else(|| "VLM report composite must be an unsigned integer.".to_string())?;
    if composite != expected_composite {
        return Err(format!(
            "VLM report composite must equal the exact subscore sum {expected_composite}."
        ));
    }
    let score = object
        .get("score")
        .and_then(Value::as_f64)
        .filter(|score| score.is_finite() && (0.0..=1.0).contains(score))
        .ok_or_else(|| "VLM report score must be finite and between 0 and 1.".to_string())?;
    // JSON numbers are parsed to IEEE-754. Requiring equality with the same
    // rational computation accepts only that canonical nearest representation;
    // no arbitrary epsilon can change a score decision.
    let expected_score = expected_composite as f64 / 9.0;
    if score != expected_score {
        return Err(format!(
            "VLM report score must equal composite/9 ({expected_score})."
        ));
    }
    let passed = object
        .get("passed")
        .and_then(Value::as_bool)
        .ok_or_else(|| "VLM report passed must be boolean.".to_string())?;
    let diagnostic = object
        .get("diagnostic")
        .and_then(Value::as_str)
        .ok_or_else(|| "VLM report diagnostic must be a string.".to_string())?;
    if diagnostic.trim().is_empty() {
        return Err("VLM report diagnostic cannot be empty.".to_string());
    }
    let failure_report = object.get("failureReport").cloned().unwrap_or(Value::Null);
    if passed {
        if score < evaluation.pass_threshold {
            return Err("A passing VLM report score is below the persisted threshold.".to_string());
        }
        if failure_report != Value::Null {
            return Err("A passing VLM report must have null failureReport.".to_string());
        }
        if has_major_problem(object) {
            return Err(
                "A passing VLM report cannot contain a major missing feature or inconsistency."
                    .to_string(),
            );
        }
        if object["enumeration"].as_array().is_none_or(Vec::is_empty) {
            return Err("A passing VLM report must enumerate requested components.".to_string());
        }
        Ok(ValidatedReport {
            report,
            score,
            passed,
            failure_report: None,
        })
    } else {
        let failure = validate_failure_report(&failure_report)?;
        Ok(ValidatedReport {
            report,
            score,
            passed,
            failure_report: Some(failure),
        })
    }
}

struct VerifiedFile {
    path: String,
    sha256: String,
}

fn verified_artifact_file(
    app_data_dir: &Path,
    artifact: &CadArtifact,
) -> Result<VerifiedFile, String> {
    let metadata = artifact
        .metadata
        .as_ref()
        .ok_or_else(|| format!("Artifact {} metadata is missing.", artifact.id))?;
    let path = metadata
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| {
            metadata
                .get("relativePath")
                .and_then(Value::as_str)
                .map(|relative| app_data_dir.join(relative))
        })
        .ok_or_else(|| format!("Artifact {} path is missing.", artifact.id))?;
    if !path.is_absolute() {
        return Err(format!("Artifact {} path must be absolute.", artifact.id));
    }
    let expected = metadata_string(artifact, "sha256")?;
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "Failed to read artifact {} at {}: {error}",
            artifact.id,
            path.display()
        )
    })?;
    if bytes.is_empty() {
        return Err(format!("Artifact {} file is empty.", artifact.id));
    }
    let actual = storage::sha256_hex(&bytes);
    if actual != expected {
        return Err(format!("Artifact {} sha256 mismatch.", artifact.id));
    }
    let path = path
        .to_str()
        .ok_or_else(|| format!("Artifact {} path is not valid UTF-8.", artifact.id))?;
    Ok(VerifiedFile {
        path: path.to_string(),
        sha256: actual,
    })
}

fn verify_contract_file(
    object: &Map<String, Value>,
    path: &Path,
    label: &str,
) -> Result<(), String> {
    let expected = required_string(object, "sha256", label)?;
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {label} file {}: {error}", path.display()))?;
    if bytes.is_empty() || storage::sha256_hex(&bytes) != expected {
        return Err(format!("{label} file/hash verification failed."));
    }
    Ok(())
}

fn validate_artifact_lineage(
    artifact: &CadArtifact,
    revision_id: &str,
    kind: CadArtifactKind,
    format: &str,
) -> Result<(), String> {
    if artifact.revision_id != revision_id || artifact.kind != kind || artifact.format != format {
        return Err(format!(
            "Artifact {} lineage/kind/format mismatch.",
            artifact.id
        ));
    }
    if artifact.deleted_at.is_some() || artifact.missing_at.is_some() {
        return Err(format!("Artifact {} is not available.", artifact.id));
    }
    Ok(())
}

fn metadata_string<'a>(artifact: &'a CadArtifact, key: &str) -> Result<&'a str, String> {
    artifact
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Artifact {} metadata is missing {key}.", artifact.id))
}

fn require_object_contract(value: &Value, label: &str) -> Result<(), String> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} must be a JSON object."))
        .map(|_| ())
}

fn require_id(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} cannot be empty."))
    } else {
        Ok(())
    }
}

fn contract_object<'a>(contract: &'a Value, field: &str) -> Result<&'a Map<String, Value>, String> {
    contract
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("VLM input contract {field} must be an object."))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{label}.{field} must be a non-empty string."))
}

fn require_exact_string(
    object: &Map<String, Value>,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = required_string(object, field, "VLM report")?;
    if actual != expected {
        return Err(format!(
            "VLM report {field} mismatch: expected {expected:?}, received {actual:?}."
        ));
    }
    Ok(())
}

fn subscore(scores: &Map<String, Value>, field: &str) -> Result<u64, String> {
    scores
        .get(field)
        .and_then(Value::as_u64)
        .filter(|score| *score <= 3)
        .ok_or_else(|| format!("VLM report scores.{field} must be an integer from 0 through 3."))
}

fn validate_findings(object: &Map<String, Value>) -> Result<(), String> {
    for finding in object["findings"].as_array().expect("array checked") {
        let finding = finding
            .as_object()
            .ok_or_else(|| "Each VLM finding must be an object.".to_string())?;
        required_string(finding, "severity", "VLM finding")?;
        required_string(finding, "message", "VLM finding")?;
    }
    for inconsistency in object["inconsistencies"].as_array().expect("array checked") {
        if !inconsistency.is_object() && !inconsistency.is_string() {
            return Err("Each VLM inconsistency must be a string or object.".to_string());
        }
    }
    Ok(())
}

fn validate_enumeration(object: &Map<String, Value>) -> Result<(), String> {
    for entry in object["enumeration"].as_array().expect("array checked") {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Each VLM enumeration entry must be an object.".to_string())?;
        required_string(entry, "planName", "VLM enumeration entry")?;
        required_string(entry, "observed", "VLM enumeration entry")?;
    }
    Ok(())
}

fn has_major_problem(object: &Map<String, Value>) -> bool {
    !object["inconsistencies"]
        .as_array()
        .expect("array checked")
        .is_empty()
        || object["findings"]
            .as_array()
            .expect("array checked")
            .iter()
            .any(|finding| {
                finding
                    .get("severity")
                    .and_then(Value::as_str)
                    .is_some_and(|severity| {
                        matches!(
                            severity.to_ascii_lowercase().as_str(),
                            "major" | "error" | "critical"
                        )
                    })
            })
        || object["enumeration"]
            .as_array()
            .expect("array checked")
            .iter()
            .any(|entry| {
                entry
                    .get("observed")
                    .and_then(Value::as_str)
                    .is_some_and(|observed| {
                        let normalized = observed.to_ascii_lowercase();
                        normalized.contains("missing") || normalized.contains("absent")
                    })
            })
}

fn validate_failure_report(value: &Value) -> Result<Value, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "A failing VLM report requires failureReport object.".to_string())?;
    require_exact_string(object, "contractType", FAILURE_REPORT_CONTRACT_TYPE)?;
    required_string(object, "reason", "failureReport")?;
    required_string(object, "summary", "failureReport")?;
    require_exact_string(object, "nextAction", "outer_loop_refine_source")?;
    Ok(value.clone())
}

pub fn validate_evaluation_kind(evaluation: &CadValidationEvaluation) -> Result<(), String> {
    if evaluation.kind != CadValidationEvaluationKind::Vlm {
        return Err(format!(
            "Unsupported validation evaluation kind: {:?}.",
            evaluation.kind
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::CadValidationEvaluationStatus;

    fn evaluation() -> CadValidationEvaluation {
        CadValidationEvaluation {
            id: "evaluation-1".to_string(),
            session_id: "session-1".to_string(),
            run_id: "run-1".to_string(),
            revision_id: "revision-1".to_string(),
            artifact_id: "artifact-1".to_string(),
            kind: CadValidationEvaluationKind::Vlm,
            attempt: 2,
            status: CadValidationEvaluationStatus::Running,
            evaluator_thread_id: Some("thread-1".to_string()),
            external_turn_id: Some("turn-1".to_string()),
            input_contract: json!({}),
            report: None,
            passed: None,
            score: None,
            pass_threshold: 0.8,
            error: None,
            created_at: "2026-08-13T00:00:00Z".to_string(),
            started_at: Some("2026-08-13T00:00:01Z".to_string()),
            completed_at: None,
        }
    }

    fn pass_report() -> Value {
        json!({
            "contractType": REPORT_CONTRACT_TYPE,
            "evaluationId": "evaluation-1",
            "sessionId": "session-1",
            "runId": "run-1",
            "revisionId": "revision-1",
            "artifactId": "artifact-1",
            "kind": "vlm",
            "attempt": 2,
            "score": 1.0,
            "passed": true,
            "scores": { "structure": 3, "components": 3, "proportions": 3 },
            "composite": 9,
            "findings": [],
            "enumeration": [{ "planName": "body", "observed": "present" }],
            "inconsistencies": [],
            "diagnostic": "All requested components are visible.",
            "failureReport": null
        })
    }

    #[test]
    fn strict_report_accepts_exact_rational_score_and_identity() {
        let validated = validate_report(&evaluation(), pass_report()).unwrap();
        assert!(validated.passed);
        assert_eq!(validated.score, 1.0);
        assert!(validated.failure_report.is_none());
    }

    #[test]
    fn strict_report_rejects_identity_and_score_contract_drift() {
        let mut identity = pass_report();
        identity["runId"] = json!("other-run");
        assert!(validate_report(&evaluation(), identity)
            .unwrap_err()
            .contains("runId mismatch"));

        let mut score = pass_report();
        score["score"] = json!(0.99);
        assert!(validate_report(&evaluation(), score)
            .unwrap_err()
            .contains("composite/9"));
    }

    #[test]
    fn strict_report_rejects_synthetic_pass_and_requires_failure_contract() {
        let mut missing = pass_report();
        missing["enumeration"][0]["observed"] = json!("major component missing");
        assert!(validate_report(&evaluation(), missing)
            .unwrap_err()
            .contains("major missing"));

        let mut failed = pass_report();
        failed["passed"] = json!(false);
        failed["score"] = json!(2.0 / 3.0);
        failed["scores"] = json!({ "structure": 2, "components": 2, "proportions": 2 });
        failed["composite"] = json!(6);
        assert!(validate_report(&evaluation(), failed)
            .unwrap_err()
            .contains("failureReport object"));
    }
}
