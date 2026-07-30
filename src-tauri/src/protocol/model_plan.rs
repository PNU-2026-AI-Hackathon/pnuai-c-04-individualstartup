use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadModelPlanComponent {
    pub name: String,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadModelAspectRatio {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub tolerance: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadModelRuntimeConstraints {
    pub runtime: CadRuntimeKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_component_annotation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadModelPlan {
    pub schema_version: String,
    pub summary: String,
    pub main_component: CadModelPlanComponent,
    #[serde(default)]
    pub supporting_components: Vec<CadModelPlanComponent>,
    pub expected_aspect_ratio: CadModelAspectRatio,
    pub source_language: CadSourceLanguage,
    pub runtime_constraints: CadModelRuntimeConstraints,
}
