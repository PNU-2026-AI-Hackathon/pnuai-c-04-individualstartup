use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadScope {
    pub session_id: String,
    pub plane: CadAgentPlane,
    pub owner_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadValidationEvaluation {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub revision_id: String,
    pub artifact_id: String,
    pub kind: CadValidationEvaluationKind,
    pub attempt: u32,
    pub status: CadValidationEvaluationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluator_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_turn_id: Option<String>,
    pub input_contract: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub pass_threshold: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadValidationEvaluationCreate {
    pub session_id: String,
    pub run_id: String,
    pub revision_id: String,
    pub artifact_id: String,
    pub kind: CadValidationEvaluationKind,
    pub input_contract: Value,
    pub pass_threshold: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadValidationEvaluationEvent {
    pub id: String,
    pub session_id: String,
    pub evaluation_id: String,
    pub evaluator_thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_item_id: Option<String>,
    pub method: String,
    pub sequence: u64,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadValidationBatch {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub revision_id: String,
    pub artifact_id: String,
    pub attempt: u32,
    pub status: CadValidationBatchStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate_report: Option<Value>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settlement_claimed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects_claimed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refinement_requested_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refinement_bound_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects_applied_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadValidationCheck {
    pub id: String,
    pub batch_id: String,
    pub session_id: String,
    pub kind: CadValidationCheckKind,
    pub status: CadValidationCheckStatus,
    pub input_contract: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluator_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_turn_id: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadValidationCheckCreate {
    pub kind: CadValidationCheckKind,
    pub input_contract: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadValidationBatchCreate {
    pub session_id: String,
    pub run_id: String,
    pub revision_id: String,
    pub artifact_id: String,
    pub checks: Vec<CadValidationCheckCreate>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadValidationCheckEvent {
    pub id: String,
    pub session_id: String,
    pub check_id: String,
    pub evaluator_thread_id: String,
    pub external_turn_id: Option<String>,
    pub external_item_id: Option<String>,
    pub method: String,
    pub sequence: u64,
    pub payload: Value,
    pub created_at: String,
}
