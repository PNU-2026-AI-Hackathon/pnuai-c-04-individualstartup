use super::*;
use crate::session_repository::SqliteSessionRepository;

mod artifact_repository;
mod atomic_run_creation;
mod operations_recovery;
mod recovery;
mod runtime_revision;
mod session_repository;
mod validation_persistence;
mod workflow_repository;

fn create_test_revision(service: &SessionService, session_id: &str, source: &str) -> String {
    let parent_revision_id = service
        .get_session_state(session_id)
        .unwrap()
        .session
        .active_revision_id;
    service
        .update_model_source(UpdateModelSourceInput {
            session_id: session_id.to_string(),
            source_language: CadSourceLanguage::Openscad,
            source: source.to_string(),
            parent_revision_id,
            parameters: None,
        })
        .unwrap()
        .revision_id
}
