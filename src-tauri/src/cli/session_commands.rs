use super::*;

pub(super) fn session_current(
    _args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let current = service.get_current_session().map_err(CliError::storage)?;
    let (active_revision_id, selected_runtime) = current
        .state
        .as_ref()
        .map(|state| {
            (
                state.session.active_revision_id.clone(),
                Some(state.session.selected_runtime.clone()),
            )
        })
        .unwrap_or((None, None));
    Ok(CommandOutput::new(json!({
        "appDataDir": app_data_dir,
        "sessionId": current.session_id,
        "uiUrl": current.ui_url,
        "activeRevisionId": active_revision_id,
        "selectedRuntime": selected_runtime,
        "state": current.state
    })))
}

pub(super) fn session_state(
    args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = resolve_session_id(args, service)?;
    let state = service
        .get_session_state(&session_id)
        .map_err(CliError::not_found)?;
    Ok(CommandOutput::new(json!({
        "appDataDir": app_data_dir,
        "sessionId": session_id,
        "state": state
    })))
}
