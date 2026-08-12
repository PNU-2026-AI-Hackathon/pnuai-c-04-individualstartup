use super::*;

impl SessionService {
    pub fn post_user_message(
        &self,
        input: PostUserMessageInput,
    ) -> Result<(CadUserMessage, CadSessionState), String> {
        let message_id = uuid();
        let created_at = timestamp();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?;
            let revision_id = input
                .revision_id
                .clone()
                .or_else(|| session.active_revision_id.clone());
            let mut event_id = None;
            if let Some(revision_id) = &revision_id {
                let revision = require_revision_mut(&mut state, revision_id)?;
                let event = add_user_event(
                    revision,
                    "message.created",
                    json!({"channel": "web-ui", "message": input.message}),
                );
                event_id = Some(event.id);
            }
            let message = CadUserMessage {
                id: message_id.clone(),
                session_id: input.session_id.clone(),
                revision_id: revision_id.clone(),
                event_id,
                channel: CadUserMessageChannel::WebUi,
                message: input.message.trim().to_string(),
                created_at: created_at.clone(),
            };
            state
                .messages
                .entry(input.session_id.clone())
                .or_default()
                .push(message);
            let conversation_message = CadConversationMessage {
                id: uuid(),
                session_id: input.session_id.clone(),
                revision_id,
                role: CadConversationRole::User,
                content: input.message.trim().to_string(),
                created_at: created_at.clone(),
                run_id: None,
                external_thread_id: None,
                external_turn_id: None,
                external_item_id: None,
                phase: None,
                sequence: None,
                is_final: true,
                metadata: Some(metadata_from_value(
                    json!({"channel": "web-ui", "legacyMessageId": message_id}),
                )),
            };
            state
                .conversation
                .entry(input.session_id.clone())
                .or_default()
                .push(conversation_message.clone());
            self.repository
                .save_conversation_message(&conversation_message)?;
            let title_updated = self.maybe_update_session_title_from_text(
                &mut state,
                &input.session_id,
                &input.message,
            )?;
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.updated_at = created_at;
            if title_updated {
                self.persist_session_graph(&state, &input.session_id)?;
            }
            build_state(&state, &input.session_id)?
        };
        let message = snapshot
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .cloned()
            .ok_or_else(|| "Message write failed.".to_string())?;
        self.emit(
            CadBridgeEventType::MessageCreated,
            &input.session_id,
            snapshot.clone(),
        );
        Ok((message, snapshot))
    }

    pub fn create_conversation_message(
        &self,
        session_id: &str,
        revision_id: Option<String>,
        role: CadConversationRole,
        content: String,
        run_id: Option<String>,
        metadata: Option<Metadata>,
    ) -> Result<(CadConversationMessage, CadSessionState), String> {
        let message_id = uuid();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, session_id)?;
            let message = CadConversationMessage {
                id: message_id.clone(),
                session_id: session_id.to_string(),
                revision_id,
                role,
                content: content.trim().to_string(),
                created_at: timestamp(),
                run_id,
                external_thread_id: None,
                external_turn_id: None,
                external_item_id: None,
                phase: None,
                sequence: None,
                is_final: true,
                metadata,
            };
            state
                .conversation
                .entry(session_id.to_string())
                .or_default()
                .push(message.clone());
            self.repository.save_conversation_message(&message)?;
            let title_updated = if message.role == CadConversationRole::User {
                self.maybe_update_session_title_from_text(&mut state, session_id, &message.content)?
            } else {
                false
            };
            if let Some(run_id) = &message.run_id {
                if matches!(
                    message.role,
                    CadConversationRole::Assistant
                        | CadConversationRole::System
                        | CadConversationRole::Tool
                ) {
                    let event = append_agent_run_event(
                        &mut state,
                        session_id,
                        run_id,
                        message.revision_id.clone(),
                        CadAgentRunEventType::AgentMessageCreated,
                        json!({
                            "messageId": message.id,
                            "role": message.role,
                            "content": message.content
                        }),
                        None,
                    );
                    persist_agent_run_event(
                        self.repository.as_ref(),
                        &mut state,
                        session_id,
                        event,
                    )?;
                }
            }
            if title_updated {
                self.persist_session_graph(&state, session_id)?;
            }
            build_state(&state, session_id)?
        };
        let message = snapshot
            .conversation
            .iter()
            .find(|message| message.id == message_id)
            .cloned()
            .ok_or_else(|| "Conversation write failed.".to_string())?;
        self.emit(
            CadBridgeEventType::AgentMessageCreated,
            session_id,
            snapshot.clone(),
        );
        Ok((message, snapshot))
    }
}
