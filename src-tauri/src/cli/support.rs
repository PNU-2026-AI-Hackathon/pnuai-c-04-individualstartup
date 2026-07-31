use crate::protocol::{
    CadModelPlan, CadModelPlanComponent, CadModelPlanDraft, CadModelRuntimeConstraints,
    CadRuntimeKind, CadSourceLanguage,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const APP_IDENTIFIER: &str = "dev.cadastrophe.desktop";
const PLAN_SCHEMA_VERSION: &str = "cad_model_plan.v1";
const PLAN_FORBIDDEN_FEATURES: &[&str] = &["external_file_include"];
const PLAN_DEFAULT_REQUIRED_FEATURE: &str = "main_component_annotation";
const PLAN_SYSTEM_OWNED_FIELDS: &[&str] =
    &["schemaVersion", "sourceLanguage", "runtimeConstraints"];

#[derive(Debug)]
pub(super) struct ParsedArgs {
    pub(super) pretty: bool,
    pub(super) values: BTreeMap<String, String>,
}

impl ParsedArgs {
    pub(super) fn app_data_dir(&self) -> CliResult<PathBuf> {
        self.optional("app-data-dir")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("CADASTROPHE_APP_DATA_DIR")
                    .ok()
                    .map(PathBuf::from)
            })
            .or_else(default_app_data_dir)
            .ok_or_else(|| {
                CliError::invalid_input(
                    "Could not determine app data directory. Pass --app-data-dir <path>.",
                )
            })
    }

    pub(super) fn required(&self, name: &str) -> CliResult<&str> {
        self.values
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| CliError::invalid_input(format!("Missing required --{name} option.")))
    }

    pub(super) fn required_path(&self, name: &str) -> CliResult<PathBuf> {
        Ok(PathBuf::from(self.required(name)?))
    }

    pub(super) fn optional(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

pub(super) fn parse_args(args: impl Iterator<Item = String>) -> CliResult<ParsedArgs> {
    let mut pretty = false;
    let mut values = BTreeMap::new();
    let mut pending_key: Option<String> = None;
    for arg in args {
        if let Some(key) = pending_key.take() {
            if arg.starts_with("--") {
                return Err(CliError::invalid_input(format!(
                    "Missing value for --{key}."
                )));
            }
            values.insert(key, arg);
            continue;
        }
        if arg == "--pretty" {
            pretty = true;
        } else if arg == "--json" {
            // JSON is the default. Accept the flag so callers can be explicit.
        } else if let Some(rest) = arg.strip_prefix("--") {
            if let Some((key, value)) = rest.split_once('=') {
                if key.is_empty() || value.is_empty() {
                    return Err(CliError::invalid_input(format!("Invalid option {arg:?}.")));
                }
                values.insert(key.to_string(), value.to_string());
            } else if rest.is_empty() {
                return Err(CliError::invalid_input("Invalid empty option --."));
            } else {
                pending_key = Some(rest.to_string());
            }
        } else {
            return Err(CliError::invalid_input(format!(
                "Unexpected positional argument {arg:?}."
            )));
        }
    }
    if let Some(key) = pending_key {
        return Err(CliError::invalid_input(format!(
            "Missing value for --{key}."
        )));
    }
    Ok(ParsedArgs { pretty, values })
}

#[derive(Debug)]
pub(super) struct CommandOutput {
    pub(super) data: Value,
    pub(super) revision_id: Option<String>,
    pub(super) event_payload: Value,
}

impl CommandOutput {
    pub(super) fn new(data: Value) -> Self {
        Self {
            data,
            revision_id: None,
            event_payload: json!({}),
        }
    }
}

pub(super) type CliResult<T> = Result<T, CliError>;

#[derive(Debug)]
pub(super) struct CliError {
    pub(super) code: &'static str,
    pub(super) message: String,
    pub(super) exit_code: i32,
}

impl CliError {
    pub(super) fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_input",
            message: message.into(),
            exit_code: 2,
        }
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "not_found",
            message: message.into(),
            exit_code: 3,
        }
    }

    pub(super) fn precondition_failed(message: impl Into<String>) -> Self {
        Self {
            code: "precondition_failed",
            message: message.into(),
            exit_code: 4,
        }
    }

    pub(super) fn storage(message: impl Into<String>) -> Self {
        Self {
            code: "storage_error",
            message: message.into(),
            exit_code: 1,
        }
    }

    pub(super) fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: "runtime_error",
            message: message.into(),
            exit_code: 5,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SuccessEnvelope {
    ok: bool,
    command: &'static str,
    data: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorEnvelope {
    ok: bool,
    command: &'static str,
    error: ErrorBody,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    code: &'static str,
    message: String,
}

pub(super) fn emit_success(command: &'static str, pretty: bool, data: Value) -> i32 {
    let envelope = SuccessEnvelope {
        ok: true,
        command,
        data,
    };
    print_json_stdout(&envelope, pretty);
    0
}

pub(super) fn emit_error(command: &'static str, pretty: bool, error: CliError) -> i32 {
    let exit_code = error.exit_code;
    let envelope = ErrorEnvelope {
        ok: false,
        command,
        error: ErrorBody {
            code: error.code,
            message: error.message,
        },
    };
    print_json_stderr(&envelope, pretty);
    exit_code
}

fn print_json_stdout(value: &impl Serialize, pretty: bool) {
    if pretty {
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("envelope serializes")
        );
    } else {
        println!(
            "{}",
            serde_json::to_string(value).expect("envelope serializes")
        );
    }
}

fn print_json_stderr(value: &impl Serialize, pretty: bool) {
    if pretty {
        eprintln!(
            "{}",
            serde_json::to_string_pretty(value).expect("envelope serializes")
        );
    } else {
        eprintln!(
            "{}",
            serde_json::to_string(value).expect("envelope serializes")
        );
    }
}

pub(super) fn merge_event_payload(base: Value, extra: Value) -> Value {
    let mut base = match base {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Value::Object(extra) = extra {
        base.extend(extra);
    }
    Value::Object(base)
}

pub(super) fn require_contract_type(
    value: &Value,
    expected: &'static str,
    label: &str,
) -> CliResult<()> {
    let actual = value.get("contractType").and_then(Value::as_str);
    if actual != Some(expected) {
        return Err(CliError::invalid_input(format!(
            "{label} contractType must be {expected}."
        )));
    }
    Ok(())
}

pub(super) fn parse_optional_f64(
    value: Option<&str>,
    default_value: f64,
    name: &str,
) -> CliResult<f64> {
    value
        .map(|value| {
            value.parse::<f64>().map_err(|error| {
                CliError::invalid_input(format!("--{name} must be a number: {error}"))
            })
        })
        .unwrap_or(Ok(default_value))
}

pub(super) fn validate_plan(plan: &CadModelPlan) -> CliResult<()> {
    if plan.schema_version != PLAN_SCHEMA_VERSION {
        return Err(CliError::invalid_input(format!(
            "Unsupported plan schemaVersion {:?}; expected {PLAN_SCHEMA_VERSION}.",
            plan.schema_version
        )));
    }
    if plan.summary.trim().is_empty() {
        return Err(CliError::invalid_input("Plan summary must not be empty."));
    }
    if plan.main_component.name.trim().is_empty() {
        return Err(CliError::invalid_input(
            "Plan mainComponent.name must not be empty.",
        ));
    }
    let ratio = &plan.expected_aspect_ratio;
    for (name, value) in [
        ("expectedAspectRatio.x", ratio.x),
        ("expectedAspectRatio.y", ratio.y),
        ("expectedAspectRatio.z", ratio.z),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(CliError::invalid_input(format!("{name} must be positive.")));
        }
    }
    if !ratio.tolerance.is_finite() || ratio.tolerance < 0.0 {
        return Err(CliError::invalid_input(
            "expectedAspectRatio.tolerance must be zero or positive.",
        ));
    }
    if plan.runtime_constraints.runtime != crate::protocol::CadRuntimeKind::OpenscadWasm {
        return Err(CliError::invalid_input(
            "Track A currently supports CadModelPlan runtimeConstraints.runtime openscad-wasm only.",
        ));
    }
    Ok(())
}

pub(super) fn parse_plan_draft_json(plan_json: &str, label: &str) -> CliResult<CadModelPlan> {
    let value: Value = serde_json::from_str(plan_json).map_err(|error| {
        CliError::invalid_input(format!(
            "Plan file {label} is not a valid JSON document: {error}",
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        CliError::invalid_input(format!(
            "Plan file {label} must be a CadModelPlanDraft JSON object."
        ))
    })?;
    let system_owned_fields = PLAN_SYSTEM_OWNED_FIELDS
        .iter()
        .copied()
        .filter(|field| object.contains_key(*field))
        .collect::<Vec<_>>();
    if !system_owned_fields.is_empty() {
        return Err(CliError::invalid_input(format!(
            "Plan draft must not define system-owned runtime policy fields: {}. cadastrophe-plan-commit owns schemaVersion, sourceLanguage, and runtimeConstraints.",
            system_owned_fields.join(", ")
        )));
    }

    let draft: CadModelPlanDraft = serde_json::from_value(value).map_err(|error| {
        CliError::invalid_input(format!(
            "Plan file {label} is not a valid CadModelPlanDraft JSON document: {error}",
        ))
    })?;
    validate_plan_draft(&draft)?;
    let plan = normalize_plan_draft(draft);
    validate_plan(&plan)?;
    Ok(plan)
}

fn validate_plan_draft(draft: &CadModelPlanDraft) -> CliResult<()> {
    if draft.summary.trim().is_empty() {
        return Err(CliError::invalid_input(
            "Plan draft summary must not be empty.",
        ));
    }
    validate_plan_component(&draft.main_component, "mainComponent")?;
    for (index, component) in draft.supporting_components.iter().enumerate() {
        validate_plan_component(component, &format!("supportingComponents[{index}]"))?;
    }
    let ratio = &draft.expected_aspect_ratio;
    for (name, value) in [
        ("expectedAspectRatio.x", ratio.x),
        ("expectedAspectRatio.y", ratio.y),
        ("expectedAspectRatio.z", ratio.z),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(CliError::invalid_input(format!("{name} must be positive.")));
        }
    }
    if !ratio.tolerance.is_finite() || ratio.tolerance < 0.0 {
        return Err(CliError::invalid_input(
            "expectedAspectRatio.tolerance must be zero or positive.",
        ));
    }
    Ok(())
}

fn validate_plan_component(component: &CadModelPlanComponent, label: &str) -> CliResult<()> {
    if component.name.trim().is_empty() {
        return Err(CliError::invalid_input(format!(
            "Plan draft {label}.name must not be empty."
        )));
    }
    if component.purpose.trim().is_empty() {
        return Err(CliError::invalid_input(format!(
            "Plan draft {label}.purpose must not be empty."
        )));
    }
    Ok(())
}

fn normalize_plan_draft(draft: CadModelPlanDraft) -> CadModelPlan {
    let main_component_name = draft.main_component.name.trim().to_string();
    let mut required_features = Vec::new();
    append_required_features(&mut required_features, &draft.main_component);
    for component in &draft.supporting_components {
        append_required_features(&mut required_features, component);
    }
    if required_features.is_empty() {
        required_features.push(PLAN_DEFAULT_REQUIRED_FEATURE.to_string());
    }

    CadModelPlan {
        schema_version: PLAN_SCHEMA_VERSION.to_string(),
        summary: draft.summary,
        main_component: draft.main_component,
        supporting_components: draft.supporting_components,
        expected_aspect_ratio: draft.expected_aspect_ratio,
        source_language: CadSourceLanguage::Openscad,
        runtime_constraints: CadModelRuntimeConstraints {
            runtime: CadRuntimeKind::OpenscadWasm,
            required_features,
            forbidden_features: PLAN_FORBIDDEN_FEATURES
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
            main_component_annotation: Some(format!("// @main_component {main_component_name}")),
        },
    }
}

fn append_required_features(
    required_features: &mut Vec<String>,
    component: &CadModelPlanComponent,
) {
    for feature in &component.required_features {
        let feature = feature.trim();
        if !feature.is_empty() && !required_features.iter().any(|existing| existing == feature) {
            required_features.push(feature.to_string());
        }
    }
}

pub(super) fn parse_source_language(value: &str) -> CliResult<CadSourceLanguage> {
    match value {
        "openscad" => Ok(CadSourceLanguage::Openscad),
        "cadquery" => Ok(CadSourceLanguage::Cadquery),
        "freecad-python" | "freecad_python" => Ok(CadSourceLanguage::FreecadPython),
        "cadastrophe-ir" | "cadastrophe_ir" => Ok(CadSourceLanguage::CadastropheIr),
        other => Err(CliError::invalid_input(format!(
            "Unsupported source language {other:?}."
        ))),
    }
}

pub(super) fn default_app_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join(APP_IDENTIFIER)
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|root| root.join(APP_IDENTIFIER))
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join(".local").join("share")))
            .map(|root| root.join(APP_IDENTIFIER))
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub(super) fn timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}.{:03}Z", chrono_like_seconds(millis), millis % 1000)
}

fn chrono_like_seconds(millis: u128) -> String {
    let seconds = millis / 1000;
    let tm = time_from_unix(seconds as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.year, tm.month, tm.day, tm.hour, tm.minute, tm.second
    )
}

struct SimpleUtcTime {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn time_from_unix(seconds: i64) -> SimpleUtcTime {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    SimpleUtcTime {
        year,
        month,
        day,
        hour: seconds_of_day / 3600,
        minute: seconds_of_day % 3600 / 60,
        second: seconds_of_day % 60,
    }
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}
