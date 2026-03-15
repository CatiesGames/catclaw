use std::sync::Arc;
use crate::agent::AgentRegistry;
use crate::channel::{
    split_at_boundaries, ChannelAdapter, MsgContext, OutboundMessage,
};
use crate::config::BindingConfig;
use crate::error::Result;
use crate::session::manager::{SenderInfo, SessionManager};
use crate::session::{Priority, SessionKey};

/// Routes inbound messages to the correct agent and session
pub struct MessageRouter {
    session_manager: Arc<SessionManager>,
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    /// Bindings from catclaw.toml
    bindings: Vec<BindingEntry>,
    default_agent_id: String,
}

#[derive(Debug, Clone)]
struct BindingEntry {
    pattern: String,
    agent_id: String,
    specificity: usize,
}

impl MessageRouter {
    pub fn new(
        session_manager: Arc<SessionManager>,
        agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
        config_bindings: &[BindingConfig],
        default_agent_id: String,
    ) -> Self {
        let mut bindings: Vec<BindingEntry> = config_bindings
            .iter()
            .map(|b| BindingEntry {
                pattern: b.pattern.clone(),
                agent_id: b.agent.clone(),
                specificity: pattern_specificity(&b.pattern),
            })
            .collect();

        // Sort by specificity (most specific first)
        bindings.sort_by(|a, b| b.specificity.cmp(&a.specificity));

        MessageRouter {
            session_manager,
            agent_registry,
            bindings,
            default_agent_id,
        }
    }

    /// Route a message: resolve agent, create/resume session, get response
    pub async fn route(
        &self,
        ctx: &MsgContext,
        adapter: &dyn ChannelAdapter,
    ) -> Result<()> {
        // 1. Start typing indicator
        let _typing = adapter.start_typing(&ctx.channel_id, &ctx.peer_id).await?;

        // 2. Resolve agent
        let agent_id = self.resolve_agent(ctx);
        let agent = {
            let registry = self.agent_registry.read().unwrap();
            registry
                .get(&agent_id)
                .or_else(|| registry.default_agent())
                .cloned()
                .ok_or_else(|| {
                    crate::error::CatClawError::Agent(format!("agent '{}' not found", agent_id))
                })?
        };

        // 3. Build session key with human-readable context_id
        let origin = ctx.channel_type.as_str();
        let context_id = if ctx.is_direct_message {
            format!("dm.{}", ctx.sender_name)
        } else if let Some(ref thread_id) = ctx.thread_id {
            let channel_name = ctx
                .channel_name
                .as_deref()
                .unwrap_or(&ctx.channel_id);
            format!("{}.thread.{}", channel_name, thread_id)
        } else {
            ctx.channel_name
                .clone()
                .unwrap_or_else(|| ctx.channel_id.clone())
        };

        let session_key = SessionKey::new(&agent.id, origin, &context_id);

        // 4. Handle /stop command — kill running session
        let text_trimmed = ctx.text.trim();
        if text_trimmed == "/stop" {
            let key_str = session_key.to_key_string();
            let stopped = self.session_manager.stop_session(&key_str);
            let reply = if stopped {
                "Session stopped.".to_string()
            } else {
                "No active session to stop.".to_string()
            };
            adapter
                .send(OutboundMessage {
                    channel_type: ctx.channel_type,
                    channel_id: ctx.channel_id.clone(),
                    peer_id: ctx.peer_id.clone(),
                    text: reply,
                    thread_id: ctx.thread_id.clone(),
                    reply_to_message_id: None,
                })
                .await?;
            return Ok(());
        }

        // 5. Determine priority
        let priority = if ctx.is_direct_message {
            Priority::Direct
        } else {
            Priority::Mention
        };

        // 6. Send to session and wait for response
        let sender = SenderInfo {
            sender_id: Some(ctx.sender_id.clone()),
            sender_name: Some(ctx.sender_name.clone()),
            channel_id: Some(ctx.channel_id.clone()),
        };
        let response = self
            .session_manager
            .send_and_wait(&session_key, &agent, &ctx.text, priority, &sender, None)
            .await?;

        // 7. Send response back through adapter (chunked if needed)
        let max_len = adapter.capabilities().max_message_length.saturating_sub(100);
        let chunks = split_at_boundaries(&response, max_len);

        for chunk in chunks {
            adapter
                .send(OutboundMessage {
                    channel_type: ctx.channel_type,
                    channel_id: ctx.channel_id.clone(),
                    peer_id: ctx.peer_id.clone(),
                    text: chunk.to_string(),
                    thread_id: ctx.thread_id.clone(),
                    reply_to_message_id: None,
                })
                .await?;
        }

        Ok(())
    }

    /// Resolve which agent handles this message using binding table
    fn resolve_agent(&self, ctx: &MsgContext) -> String {
        let channel_type = ctx.channel_type.as_str();

        // Build candidate patterns from most specific to least
        let candidates = vec![
            // Thread-specific
            ctx.thread_id.as_ref().map(|t| {
                format!("{}:channel:{}:thread:{}", channel_type, ctx.channel_id, t)
            }),
            // Channel-specific
            Some(format!("{}:channel:{}", channel_type, ctx.channel_id)),
            // Guild-specific (from raw_event)
            ctx.raw_event
                .get("guild_id")
                .and_then(|v| v.as_str())
                .map(|g| format!("{}:guild:{}", channel_type, g)),
            // Platform wildcard
            Some(format!("{}:*", channel_type)),
            // Global wildcard
            Some("*".to_string()),
        ];

        // Find the most specific matching binding
        for candidate in candidates.into_iter().flatten() {
            for binding in &self.bindings {
                if binding.pattern == candidate {
                    return binding.agent_id.clone();
                }
            }
        }

        self.default_agent_id.clone()
    }
}

/// Calculate pattern specificity (more colons = more specific)
fn pattern_specificity(pattern: &str) -> usize {
    if pattern == "*" {
        return 0;
    }
    pattern.matches(':').count() + 1
}
