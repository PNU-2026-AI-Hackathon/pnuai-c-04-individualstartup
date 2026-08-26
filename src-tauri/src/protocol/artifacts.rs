use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadArtifact {
    pub id: String,
    pub revision_id: String,
    pub revision_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_hash: Option<String>,
    pub kind: CadArtifactKind,
    pub format: String,
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadPreviewResult {
    pub diagnostics: CadDiagnostics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh: Option<CadMesh>,
    pub artifacts: Vec<CadArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadExportResult {
    pub diagnostics: CadDiagnostics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<CadArtifact>,
}
