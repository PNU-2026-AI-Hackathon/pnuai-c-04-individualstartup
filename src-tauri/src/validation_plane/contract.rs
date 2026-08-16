use crate::protocol::{
    CadArtifact, CadArtifactKind, CadValidationEvaluation, CadValidationEvaluationKind,
};
use crate::storage;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const INPUT_CONTRACT_TYPE: &str = "cadastrophe.vlm_evaluation_input.v1";
pub const SUBMISSION_CONTRACT_TYPE: &str = "cadastrophe.vlm_submission.v1";
pub const REPORT_CONTRACT_TYPE: &str = "cadastrophe.vlm_judge_report.v1";
pub const FAILURE_REPORT_CONTRACT_TYPE: &str = "cadastrophe.failure_report.v1";
pub const VLM_PASS_COMPOSITE: u64 = 7;
pub const VLM_MIN_SUBSCORE: u64 = 2;
pub const VLM_PASS_THRESHOLD: f64 = VLM_PASS_COMPOSITE as f64 / 9.0;

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
    if input.pass_threshold != VLM_PASS_THRESHOLD {
        return Err(format!(
            "VLM evaluation passThreshold must be the system-owned threshold {VLM_PASS_THRESHOLD}."
        ));
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
        "rubric": {
            "subscoreRange": { "minimum": 0, "maximum": 3 },
            "composite": "scores.structure + scores.components + scores.proportions",
            "score": "composite / 9",
            "systemPassRule": "composite >= 7 and every subscore >= 2",
            "passComputedBy": "receiving_system"
        },
        "submissionContract": {
            "contractType": SUBMISSION_CONTRACT_TYPE,
            "command": "cadastrophe-vlm-submit",
            "requiredOptions": ["structure", "components", "proportions"],
            "optionalOptions": ["inconsistency", "diagnostic"],
            "applicationOwnedReportFields": ["evaluationId", "sessionId", "runId", "revisionId", "artifactId", "kind", "attempt", "composite", "score", "passed", "failureReport"]
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
    for field in ["composite", "score", "passed", "failureReport"] {
        if object.contains_key(field) {
            return Err(format!(
                "VLM evaluator must not set application-owned field {field}."
            ));
        }
    }
    for field in ["findings", "enumeration", "inconsistencies"] {
        if object.get(field).is_some_and(|value| !value.is_array()) {
            return Err(format!(
                "VLM report {field} must be an array when provided."
            ));
        }
    }
    validate_findings(object)?;
    validate_enumeration(object)?;
    let scores = object
        .get("scores")
        .and_then(Value::as_object)
        .ok_or_else(|| "VLM report scores must be an object.".to_string())?;
    validate_exact_scores(scores, "VLM report")?;
    let structure = subscore(scores, "structure")?;
    let components = subscore(scores, "components")?;
    let proportions = subscore(scores, "proportions")?;
    if let Some(diagnostic) = object.get("diagnostic") {
        let diagnostic = diagnostic
            .as_str()
            .ok_or_else(|| "VLM report diagnostic must be a string when provided.".to_string())?;
        if diagnostic.trim().is_empty() {
            return Err("VLM report diagnostic cannot be empty when provided.".to_string());
        }
    }
    let composite = structure + components + proportions;
    let score = composite as f64 / 9.0;
    let passed = composite >= VLM_PASS_COMPOSITE
        && [structure, components, proportions]
            .into_iter()
            .all(|subscore| subscore >= VLM_MIN_SUBSCORE);
    let failure_report = (!passed).then(|| {
        json!({
            "contractType": FAILURE_REPORT_CONTRACT_TYPE,
            "reason": "vlm_score_gate_failed",
            "summary": format!(
                "VLM score gate requires a composite of at least {VLM_PASS_COMPOSITE}/9 and every subscore at least {VLM_MIN_SUBSCORE}; received structure={structure}, components={components}, proportions={proportions}, composite={composite}/9."
            ),
            "nextAction": "outer_loop_refine_source"
        })
    });
    let mut annotated = object.clone();
    annotated.insert("composite".to_string(), json!(composite));
    annotated.insert("score".to_string(), json!(score));
    annotated.insert("passed".to_string(), json!(passed));
    annotated.insert(
        "failureReport".to_string(),
        failure_report.clone().unwrap_or(Value::Null),
    );
    Ok(ValidatedReport {
        report: Value::Object(annotated),
        score,
        passed,
        failure_report,
    })
}

pub fn build_report_from_submission(
    evaluation: &CadValidationEvaluation,
    submission: Value,
) -> Result<Value, String> {
    let object = submission
        .as_object()
        .ok_or_else(|| "VLM CLI submission must be a JSON object.".to_string())?;
    if object.get("contractType").and_then(Value::as_str) != Some(SUBMISSION_CONTRACT_TYPE) {
        return Err(format!(
            "VLM CLI submission contractType must be {SUBMISSION_CONTRACT_TYPE}."
        ));
    }
    if let Some(field) = object.keys().find(|field| {
        !matches!(
            field.as_str(),
            "contractType" | "scores" | "inconsistencies" | "diagnostic"
        )
    }) {
        return Err(format!(
            "VLM CLI submission contains unsupported field {field}."
        ));
    }
    let scores = object
        .get("scores")
        .and_then(Value::as_object)
        .ok_or_else(|| "VLM CLI submission scores must be an object.".to_string())?;
    validate_exact_scores(scores, "VLM CLI submission")?;
    let inconsistencies = match object.get("inconsistencies") {
        None => None,
        Some(value) => {
            let values = value.as_array().ok_or_else(|| {
                "VLM CLI submission inconsistencies must be an array.".to_string()
            })?;
            if values.len() != 1
                || values[0]
                    .as_str()
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err(
                    "VLM CLI submission inconsistencies must contain one non-empty string."
                        .to_string(),
                );
            }
            Some(value.clone())
        }
    };
    let diagnostic = match object.get("diagnostic") {
        None => None,
        Some(value) => {
            if value
                .as_str()
                .is_none_or(|diagnostic| diagnostic.trim().is_empty())
            {
                return Err("VLM CLI submission diagnostic must be a non-empty string.".to_string());
            }
            Some(value.clone())
        }
    };
    let mut report = Map::from_iter([
        ("contractType".to_string(), json!(REPORT_CONTRACT_TYPE)),
        ("evaluationId".to_string(), json!(evaluation.id)),
        ("sessionId".to_string(), json!(evaluation.session_id)),
        ("runId".to_string(), json!(evaluation.run_id)),
        ("revisionId".to_string(), json!(evaluation.revision_id)),
        ("artifactId".to_string(), json!(evaluation.artifact_id)),
        ("kind".to_string(), json!("vlm")),
        ("attempt".to_string(), json!(evaluation.attempt)),
        ("scores".to_string(), Value::Object(scores.clone())),
    ]);
    if let Some(inconsistencies) = inconsistencies {
        report.insert("inconsistencies".to_string(), inconsistencies);
    }
    if let Some(diagnostic) = diagnostic {
        report.insert("diagnostic".to_string(), diagnostic);
    }
    Ok(Value::Object(report))
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

fn validate_exact_scores(scores: &Map<String, Value>, label: &str) -> Result<(), String> {
    if scores.len() != 3
        || scores
            .keys()
            .any(|field| !matches!(field.as_str(), "structure" | "components" | "proportions"))
    {
        return Err(format!(
            "{label} scores must contain exactly structure, components, and proportions."
        ));
    }
    for field in ["structure", "components", "proportions"] {
        subscore(scores, field)?;
    }
    Ok(())
}

fn validate_findings(object: &Map<String, Value>) -> Result<(), String> {
    for finding in object
        .get("findings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let finding = finding
            .as_object()
            .ok_or_else(|| "Each VLM finding must be an object.".to_string())?;
        required_string(finding, "severity", "VLM finding")?;
        required_string(finding, "message", "VLM finding")?;
    }
    for inconsistency in object
        .get("inconsistencies")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if !inconsistency.is_object() && !inconsistency.is_string() {
            return Err("Each VLM inconsistency must be a string or object.".to_string());
        }
    }
    Ok(())
}

fn validate_enumeration(object: &Map<String, Value>) -> Result<(), String> {
    for entry in object
        .get("enumeration")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let entry = entry
            .as_object()
            .ok_or_else(|| "Each VLM enumeration entry must be an object.".to_string())?;
        required_string(entry, "planName", "VLM enumeration entry")?;
        required_string(entry, "observed", "VLM enumeration entry")?;
    }
    Ok(())
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
            pass_threshold: VLM_PASS_THRESHOLD,
            error: None,
            created_at: "2026-08-13T00:00:00Z".to_string(),
            started_at: Some("2026-08-13T00:00:01Z".to_string()),
            completed_at: None,
        }
    }

    fn evaluator_report(scores: [u64; 3]) -> Value {
        json!({
            "contractType": REPORT_CONTRACT_TYPE,
            "evaluationId": "evaluation-1",
            "sessionId": "session-1",
            "runId": "run-1",
            "revisionId": "revision-1",
            "artifactId": "artifact-1",
            "kind": "vlm",
            "attempt": 2,
            "scores": {
                "structure": scores[0],
                "components": scores[1],
                "proportions": scores[2]
            }
        })
    }

    #[test]
    fn receiver_derives_score_and_passed_from_evaluator_subscores() {
        let validated = validate_report(&evaluation(), evaluator_report([2, 2, 3])).unwrap();
        assert!(validated.passed);
        assert_eq!(validated.score, 7.0 / 9.0);
        assert!(validated.failure_report.is_none());
        assert_eq!(validated.report["composite"], 7);
        assert_eq!(validated.report["score"], 7.0 / 9.0);
        assert_eq!(validated.report["passed"], true);
        assert_eq!(validated.report["failureReport"], Value::Null);
    }

    #[test]
    fn strict_report_rejects_identity_drift_and_evaluator_owned_passed() {
        let mut identity = evaluator_report([3, 3, 3]);
        identity["runId"] = json!("other-run");
        assert!(validate_report(&evaluation(), identity)
            .unwrap_err()
            .contains("runId mismatch"));

        let mut passed = evaluator_report([3, 3, 3]);
        passed["passed"] = json!(true);
        assert!(validate_report(&evaluation(), passed)
            .unwrap_err()
            .contains("application-owned field passed"));
    }

    #[test]
    fn receiver_requires_both_composite_and_per_item_score_gates() {
        let below_composite = validate_report(&evaluation(), evaluator_report([2, 2, 2])).unwrap();
        assert!(!below_composite.passed);
        assert_eq!(below_composite.report["composite"], 6);
        assert_eq!(below_composite.report["passed"], false);
        assert_eq!(
            below_composite.report["failureReport"]["reason"],
            "vlm_score_gate_failed"
        );

        let low_item = validate_report(&evaluation(), evaluator_report([1, 3, 3])).unwrap();
        assert_eq!(low_item.report["composite"], 7);
        assert!(!low_item.passed);
    }

    #[test]
    fn inconsistencies_are_optional_and_do_not_override_numeric_pass() {
        let mut report = evaluator_report([3, 2, 2]);
        report["inconsistencies"] = json!(["The rear view appears slightly asymmetric."]);
        report["findings"] = json!([{
            "severity": "error",
            "message": "A visible mismatch remains, recorded independently of scoring."
        }]);
        let validated = validate_report(&evaluation(), report).unwrap();
        assert!(validated.passed);
        assert_eq!(
            validated.report["inconsistencies"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
