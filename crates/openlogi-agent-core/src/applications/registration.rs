//! Registration leases own context catalogs, live parameter values and request validation.

use std::collections::BTreeMap;
use std::sync::Arc;

use openlogi_sdk::{ClientMessage, DeactivationReason, ErrorCode, Provider};

use super::{ApplicationHost, HostError, Outbox, Registered, State};

impl ApplicationHost {
    pub(super) fn register(
        self: &Arc<Self>,
        provider: Provider,
    ) -> Result<Registration, HostError> {
        let mut state = self.state();
        if state.providers.contains_key(&provider.id) {
            return Err(HostError::new(
                ErrorCode::DuplicateProvider,
                "provider already has a live connection",
            ));
        }
        let connection = state.next_id();
        let queue = Arc::new(Outbox::default());
        state.providers.insert(
            provider.id.clone(),
            Registered {
                info: provider,
                connection,
                revision: 0,
                tools: None,
                values: BTreeMap::new(),
                queue: Arc::clone(&queue),
            },
        );
        state.topology += 1;
        self.publish(&state);
        Ok(Registration {
            host: Arc::clone(self),
            connection,
            queue,
        })
    }

    pub(super) fn handle(&self, connection: u64, message: ClientMessage) -> Result<(), HostError> {
        let mut state = self.state();
        let State {
            providers,
            active,
            parameters,
            ..
        } = &mut *state;
        let provider = providers
            .values_mut()
            .find(|provider| provider.connection == connection)
            .ok_or(HostError::new(
                ErrorCode::StaleSession,
                "connection lease has ended",
            ))?;
        match message {
            ClientMessage::Register { .. } => {
                return Err(HostError::new(
                    ErrorCode::InvalidMessage,
                    "connection is already registered",
                ));
            }
            ClientMessage::Context { revision, tools } => {
                if revision <= provider.revision {
                    return Err(HostError::new(
                        ErrorCode::StaleContext,
                        "context revision must increase",
                    ));
                }
                provider.revision = revision;
                provider.values = tools
                    .as_ref()
                    .map(|tools| {
                        tools
                            .tools()
                            .iter()
                            .filter_map(|tool| Some((tool.id.clone(), tool.parameter.clone()?)))
                            .collect()
                    })
                    .unwrap_or_default();
                provider.tools = tools;
                let ended = active
                    .iter()
                    .filter(|(_, active)| active.lease.connection == connection)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for session in ended {
                    state.end(session, DeactivationReason::ContextReplaced);
                }
                state.topology += 1;
            }
            ClientMessage::Parameter {
                session,
                context_revision,
                tool,
                value,
            } => {
                if provider.revision != context_revision {
                    return Err(HostError::new(
                        ErrorCode::StaleContext,
                        "parameter context has ended",
                    ));
                }
                active
                    .get(&session)
                    .filter(|active| {
                        active.lease.connection == connection
                            && active.lease.revision == context_revision
                    })
                    .ok_or(HostError::new(
                        ErrorCode::StaleSession,
                        "parameter activation has ended",
                    ))?;
                let current = provider.values.get_mut(&tool).ok_or(HostError::new(
                    ErrorCode::UnknownTool,
                    "tool is unknown or has no parameter",
                ))?;
                if *current != value {
                    *current = value;
                    *parameters += 1;
                }
            }
        }
        self.publish(&state);
        Ok(())
    }

    pub(super) fn disconnect_all(&self) {
        let mut state = self.state();
        let connections = state
            .providers
            .values()
            .map(|provider| provider.connection)
            .collect::<Vec<_>>();
        for connection in connections {
            state.disconnect(connection);
        }
        self.publish(&state);
    }
}

pub(super) struct Registration {
    host: Arc<ApplicationHost>,
    pub(super) connection: u64,
    pub(super) queue: Arc<Outbox>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut state = self.host.state();
        state.disconnect(self.connection);
        self.host.publish(&state);
    }
}
