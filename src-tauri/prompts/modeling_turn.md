# CADGEN-AX modeling turn

## User request

The JSON string below is the complete user request for this turn.

```json
{{USER_REQUEST_JSON}}
```

## Exact scoped CLI commands

- Session state: `{{SESSION_STATE_COMMAND}}`
- Plan commit: `{{PLAN_COMMIT_COMMAND}}`
- Source apply: `{{SOURCE_APPLY_COMMAND}}`
- Finalize: `{{FINALIZE_COMMAND}}`

## Immutable app context for this turn

```json
{
  "appDataDir": {{APP_DATA_DIR_JSON}},
  "sessionId": {{SESSION_ID_JSON}},
  "runId": {{RUN_ID_JSON}},
  "inputRevisionId": {{INPUT_REVISION_ID_JSON}},
  "activeRevisionSourceLanguage": {{SOURCE_LANGUAGE_JSON}},
  "latestWorkflowFailureReport": {{FAILURE_REPORT_JSON}},
  "activeRevisionSource": {{SOURCE_JSON}}
}
```
