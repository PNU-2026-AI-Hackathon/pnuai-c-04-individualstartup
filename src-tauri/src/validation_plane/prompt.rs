use crate::prompt_template::render_strict_template;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const VLM_PROMPT_TEMPLATE: &str = include_str!("../../prompts/vlm_evaluator.md");
const TEMPLATE_NAME: &str = "vlm_evaluator.md";
const PLACEHOLDERS: &[&str] = &["EVALUATION_CONTRACT_JSON"];

#[derive(Clone, Copy, Debug)]
pub struct ValidationPromptContext<'a> {
    pub evaluation_id: &'a str,
    pub session_id: &'a str,
    pub run_id: &'a str,
    pub revision_id: &'a str,
    pub artifact_id: &'a str,
    pub evaluation_contract: &'a Value,
}

pub fn render_vlm_evaluator_prompt(
    context: &ValidationPromptContext<'_>,
) -> Result<String, String> {
    require_non_empty("evaluation ID", context.evaluation_id)?;
    require_non_empty("session ID", context.session_id)?;
    require_non_empty("run ID", context.run_id)?;
    require_non_empty("revision ID", context.revision_id)?;
    require_non_empty("artifact ID", context.artifact_id)?;

    let contract = context
        .evaluation_contract
        .as_object()
        .ok_or_else(|| "VLM evaluation input contract must be a JSON object.".to_string())?;
    require_contract_string(contract, "userRequest")?;
    require_matching_contract_id(contract, "evaluationId", context.evaluation_id)?;
    require_matching_contract_id(contract, "sessionId", context.session_id)?;
    require_matching_contract_id(contract, "runId", context.run_id)?;
    require_matching_contract_id(contract, "revisionId", context.revision_id)?;
    require_matching_contract_id(contract, "artifactId", context.artifact_id)?;

    let contract_json = serde_json::to_string_pretty(context.evaluation_contract)
        .map_err(|error| format!("Failed to serialize VLM evaluation input contract: {error}"))?;
    render_strict_template(
        TEMPLATE_NAME,
        VLM_PROMPT_TEMPLATE,
        PLACEHOLDERS,
        vec![("EVALUATION_CONTRACT_JSON", contract_json)],
    )
}

pub fn build_validation_thread_start_params(cwd: &Path) -> Result<Value, String> {
    validate_directory("validation working directory", cwd)?;
    Ok(json!({
        "approvalPolicy": "never",
        "cwd": cwd,
        "personality": "pragmatic",
        "sandbox": "read-only",
        "serviceName": "cadgen-ax-tauri-backend",
        "sessionStartSource": "startup"
    }))
}

pub fn build_validation_turn_start_params(
    prompt: &str,
    image_path: &Path,
    cwd: &Path,
    app_data_dir: &Path,
) -> Result<Value, String> {
    require_non_empty("rendered VLM prompt", prompt)?;
    validate_directory("validation working directory", cwd)?;
    validate_directory("CADGEN-AX app-data directory", app_data_dir)?;
    validate_rendered_image(image_path)?;
    let image_path = path_text("rendered image path", image_path)?;

    Ok(json!({
        "input": [
            {
                "type": "text",
                "text": prompt,
                "text_elements": []
            },
            {
                "type": "localImage",
                "path": image_path
            }
        ],
        "personality": "pragmatic",
        "approvalPolicy": "never",
        "cwd": cwd,
        "sandboxPolicy": {
            "type": "readOnly",
            "networkAccess": false
        }
    }))
}

fn require_contract_string(
    contract: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<(), String> {
    let value = contract
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("VLM evaluation input contract is missing string field {field}."))?;
    if value.trim().is_empty() {
        return Err(format!(
            "VLM evaluation input contract field {field} cannot be empty."
        ));
    }
    Ok(())
}

fn require_matching_contract_id(
    contract: &serde_json::Map<String, Value>,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    require_contract_string(contract, field)?;
    let actual = contract[field]
        .as_str()
        .expect("string field was validated immediately above");
    if actual != expected {
        return Err(format!(
            "VLM evaluation input contract {field} mismatch: expected {expected:?}, received {actual:?}."
        ));
    }
    Ok(())
}

fn validate_directory(label: &str, path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path."));
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Failed to access {label} {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!("{label} is not a directory: {}.", path.display()));
    }
    path_text(label, path)?;
    Ok(())
}

fn validate_rendered_image(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("Rendered VLM image path must be absolute.".to_string());
    }
    let metadata = fs::metadata(path).map_err(|error| {
        format!(
            "Failed to access rendered VLM image {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "Rendered VLM image path is not a file: {}.",
            path.display()
        ));
    }
    if metadata.len() == 0 {
        return Err(format!(
            "Rendered VLM image file is empty: {}.",
            path.display()
        ));
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("png") {
        return Err(format!(
            "Rendered VLM image must be a PNG file: {}.",
            path.display()
        ));
    }
    path_text("rendered VLM image path", path)?;
    Ok(())
}

fn path_text<'a>(label: &str, path: &'a Path) -> Result<&'a str, String> {
    path.to_str()
        .ok_or_else(|| format!("{label} is not valid UTF-8."))
}

fn require_non_empty(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} cannot be empty."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_validation_thread_start_params, build_validation_turn_start_params,
        render_vlm_evaluator_prompt, ValidationPromptContext,
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::path::{Path, PathBuf};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("cadgen-ax-prompt-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn contract() -> Value {
        json!({
            "contractType": "cadgen-ax.vlm_evaluation_input.v1",
            "evaluationId": "evaluation-1",
            "sessionId": "session-1",
            "runId": "run-1",
            "revisionId": "revision-1",
            "artifactId": "artifact-1",
            "userRequest": "Create a wall bracket.",
            "passThreshold": 7.0 / 9.0,
            "renderedImage": {
                "artifactId": "render-artifact-1",
                "mediaType": "image/png"
            },
            "submissionContract": "cadgen-ax.vlm_submission.v1"
        })
    }

    fn context<'a>(contract: &'a Value) -> ValidationPromptContext<'a> {
        ValidationPromptContext {
            evaluation_id: "evaluation-1",
            session_id: "session-1",
            run_id: "run-1",
            revision_id: "revision-1",
            artifact_id: "artifact-1",
            evaluation_contract: contract,
        }
    }

    #[test]
    fn vlm_prompt_contains_only_the_app_evaluation_contract_and_strict_output_rules() {
        let contract = contract();
        let prompt = render_vlm_evaluator_prompt(&context(&contract)).unwrap();
        assert!(prompt.starts_with("# CADGEN-AX app-owned VLM evaluator\n"));
        assert!(prompt.contains("\"evaluationId\": \"evaluation-1\""));
        assert!(prompt.contains("\"userRequest\": \"Create a wall bracket.\""));
        assert!(prompt.contains("single rendered image attached to this\nturn"));
        assert!(prompt.contains("cadgen-ax-vlm-submit --components <0-3>"));
        assert!(prompt.contains("The CLI call is the only submission"));
        assert!(prompt.contains("Do not place scores or report JSON in your"));
        assert!(prompt
            .contains("inconsistency may be submitted even when the numeric score later passes"));
        assert!(!prompt.contains("Codex Skill invocation"));
        assert!(!prompt.contains("{{EVALUATION_CONTRACT_JSON}}"));
    }

    #[test]
    fn vlm_prompt_rejects_missing_user_request_and_identity_mismatch() {
        let mut missing_user_contract = contract();
        missing_user_contract
            .as_object_mut()
            .unwrap()
            .remove("userRequest");
        assert!(
            render_vlm_evaluator_prompt(&context(&missing_user_contract))
                .unwrap_err()
                .contains("missing string field userRequest")
        );

        let mut mismatched_contract = contract();
        mismatched_contract["revisionId"] = json!("wrong-revision");
        assert!(render_vlm_evaluator_prompt(&context(&mismatched_contract))
            .unwrap_err()
            .contains("revisionId mismatch"));
    }

    #[test]
    fn validation_start_json_is_fresh_and_has_exact_text_plus_local_image_contract() {
        let temp = TestDirectory::new();
        let image_path = temp.path().join("render-grid.png");
        fs::write(&image_path, b"png evidence").unwrap();

        let thread = build_validation_thread_start_params(temp.path()).unwrap();
        assert_eq!(
            thread,
            json!({
                "approvalPolicy": "never",
                "cwd": temp.path(),
                "personality": "pragmatic",
                "sandbox": "read-only",
                "serviceName": "cadgen-ax-tauri-backend",
                "sessionStartSource": "startup"
            })
        );
        assert!(thread.get("threadId").is_none());

        let turn = build_validation_turn_start_params(
            "rendered evaluation prompt",
            &image_path,
            temp.path(),
            temp.path(),
        )
        .unwrap();
        assert_eq!(
            turn["input"],
            json!([
                {
                    "type": "text",
                    "text": "rendered evaluation prompt",
                    "text_elements": []
                },
                {
                    "type": "localImage",
                    "path": image_path
                }
            ])
        );
        assert_eq!(turn["input"].as_array().unwrap().len(), 2);
        assert_eq!(turn["sandboxPolicy"]["networkAccess"], false);
        assert_eq!(turn["sandboxPolicy"]["type"], "readOnly");
        assert!(turn["sandboxPolicy"].get("writableRoots").is_none());
        assert!(turn.get("threadId").is_none());
    }

    #[test]
    fn validation_turn_rejects_missing_or_relative_image() {
        let temp = TestDirectory::new();
        let relative = Path::new("render-grid.png");
        assert!(
            build_validation_turn_start_params("prompt", relative, temp.path(), temp.path())
                .unwrap_err()
                .contains("must be absolute")
        );

        let missing = temp.path().join("missing.png");
        assert!(
            build_validation_turn_start_params("prompt", &missing, temp.path(), temp.path())
                .unwrap_err()
                .contains("Failed to access rendered VLM image")
        );
    }
}
