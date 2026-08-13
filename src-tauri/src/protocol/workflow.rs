use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadWorkflowPlan {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    pub plan: CadModelPlan,
    pub source_language: CadSourceLanguage,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadWorkflowOuterIteration {
    pub id: String,
    pub run_id: String,
    pub iteration: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    pub structural_report: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dfm_report: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlm_report: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_report: Option<Value>,
    pub passed: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadWorkflowPendingVlm {
    pub run_id: String,
    pub artifact_id: String,
    #[serde(skip)]
    pub revision_id: Option<String>,
    pub contract: Value,
    pub pass_threshold: f64,
    #[serde(skip)]
    pub structural_report: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dfm_report: Option<Value>,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadWorkflowState {
    pub plans: Vec<CadWorkflowPlan>,
    pub outer_iterations: Vec<CadWorkflowOuterIteration>,
    pub pending_vlm: Vec<CadWorkflowPendingVlm>,
}
