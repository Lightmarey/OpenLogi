//! Application registration leases and device-independent control routing.
//!
//! The host never performs HID I/O or OS input synthesis. Adapters supply real
//! capabilities and inputs, while saved configuration decides provider scope.

mod crown;
mod queue;
mod registration;
mod server;
#[cfg(test)]
pub(crate) mod tests;

pub(crate) use crown::apply_crown_contexts;
pub(crate) use crown::motion_for as crown_motion_for;
#[cfg(test)]
pub(crate) use crown::profile_for as crown_profile_for;
pub use server::ApplicationServer;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use openlogi_core::application_control::ProviderDescriptor;
use openlogi_sdk::{
    Activation, ControlCapabilities, ControlId, ControlInput, ControlSource, DeactivationReason,
    ErrorCode, ParameterState, Provider, ProviderId, SelectionInput, ServerMessage, SessionId,
    ToolSet,
};
use tokio::sync::watch;

use self::queue::Outbox;

/// A resolved provider context. Configuration keeps its independent local fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationContext {
    provider: ProviderId,
    connection: u64,
    revision: u64,
    tools: ToolSet,
}

impl ApplicationContext {
    pub(crate) const fn provider(&self) -> &ProviderId {
        &self.provider
    }

    /// Current ordered application tools.
    #[must_use]
    pub const fn tools(&self) -> &ToolSet {
        &self.tools
    }
}

/// Agent-local binding scope. These identities never leave the host.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ControlScope {
    source: String,
    application: Option<String>,
}

impl ControlScope {
    /// Name a source in its effective application scope.
    #[must_use]
    pub const fn new(source: String, application: Option<String>) -> Self {
        Self {
            source,
            application,
        }
    }
}

/// An exact activation capability, invalidated on focus, context or connection change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveSession {
    id: SessionId,
    connection: u64,
    revision: u64,
}

impl ActiveSession {
    /// Opaque identity sent to the application.
    #[must_use]
    pub const fn id(self) -> SessionId {
        self.id
    }
    /// Context revision required by the application's parameter updates.
    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }
}

struct Registered {
    info: Provider,
    connection: u64,
    revision: u64,
    tools: Option<ToolSet>,
    values: BTreeMap<ControlId, ParameterState>,
    queue: Arc<Outbox>,
}

struct Active {
    lease: ActiveSession,
    scope: ControlScope,
    context: ApplicationContext,
    selected: ControlId,
    sequence: u64,
}

struct State {
    providers: BTreeMap<ProviderId, Registered>,
    allowed: BTreeMap<ControlScope, ApplicationContext>,
    sources: BTreeMap<String, ControlId>,
    active: BTreeMap<SessionId, Active>,
    counter: u64,
    topology: u64,
    parameters: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            providers: BTreeMap::new(),
            allowed: BTreeMap::new(),
            sources: BTreeMap::new(),
            active: BTreeMap::new(),
            counter: 0,
            topology: 1,
            parameters: 1,
        }
    }
}

impl State {
    fn next_id(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    fn disconnect(&mut self, connection: u64) {
        let provider = self
            .providers
            .iter()
            .find(|(_, provider)| provider.connection == connection)
            .map(|(id, _)| id.clone());
        if let Some(provider) = provider.and_then(|id| self.providers.remove(&id)) {
            provider.queue.close();
            self.active
                .retain(|_, active| active.lease.connection != connection);
            self.topology += 1;
            self.parameters += 1;
        }
    }

    fn emit(&mut self, connection: u64, message: ServerMessage) -> bool {
        let queue = self
            .providers
            .values()
            .find(|provider| provider.connection == connection)
            .map(|provider| Arc::clone(&provider.queue));
        if queue.is_some_and(|queue| queue.push(message)) {
            return true;
        }
        self.disconnect(connection);
        false
    }

    fn end(&mut self, session: SessionId, reason: DeactivationReason) {
        if let Some(active) = self.active.remove(&session) {
            self.emit(
                active.lease.connection,
                ServerMessage::Deactivated { session, reason },
            );
            self.parameters += 1;
        }
    }
}

/// A host refusal leaves the current context and activation intact.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct HostError {
    code: ErrorCode,
    message: &'static str,
}

impl HostError {
    const fn new(code: ErrorCode, message: &'static str) -> Self {
        Self { code, message }
    }
    fn response(&self) -> ServerMessage {
        ServerMessage::Error {
            code: self.code,
            message: self.message.into(),
        }
    }
}

/// Agent-owned application state. Input methods only validate, mutate and enqueue.
pub struct ApplicationHost {
    state: Mutex<State>,
    started: Instant,
    changed: watch::Sender<u64>,
    parameters: watch::Sender<u64>,
    directory: watch::Sender<openlogi_ipc::ApplicationProvidersObservation>,
}

impl Default for ApplicationHost {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            started: Instant::now(),
            changed: watch::channel(1).0,
            parameters: watch::channel(1).0,
            directory: watch::channel(openlogi_ipc::ApplicationProvidersObservation::new(
                1,
                Vec::new(),
            ))
            .0,
        }
    }
}

impl ApplicationHost {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, state: &State) {
        if self.changed.send_if_modified(|revision| {
            if *revision == state.topology {
                return false;
            }
            *revision = state.topology;
            true
        }) {
            let providers = state
                .providers
                .values()
                .map(|provider| {
                    ProviderDescriptor::new(
                        provider.info.id.clone(),
                        provider.info.name.as_ref().to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            self.directory.send_if_modified(|directory| {
                if directory.providers == providers {
                    return false;
                }
                directory.providers = providers;
                directory.generation += 1;
                true
            });
        }
        self.parameters.send_if_modified(|revision| {
            if *revision == state.parameters {
                return false;
            }
            *revision = state.parameters;
            true
        });
    }

    /// Notify the orchestrator when connection or context resolution changes.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Notify a read-only presenter when an application publishes a new value.
    #[must_use]
    pub fn subscribe_parameters(&self) -> watch::Receiver<u64> {
        self.parameters.subscribe()
    }

    /// Observe online providers using the internal IPC's level-triggered contract.
    pub async fn observe_providers(
        &self,
        since: openlogi_ipc::Generation,
    ) -> openlogi_ipc::ApplicationProvidersObservation {
        let mut directory = self.directory.subscribe();
        if directory.borrow().generation == since {
            let _ = tokio::time::timeout(openlogi_ipc::OBSERVE_HOLD, directory.changed()).await;
        }
        directory.borrow().clone()
    }

    /// Resolve only the provider explicitly selected by saved configuration.
    #[must_use]
    pub fn resolve(&self, id: &ProviderId) -> Option<ApplicationContext> {
        let state = self.state();
        let provider = state.providers.get(id)?;
        Some(ApplicationContext {
            provider: id.clone(),
            connection: provider.connection,
            revision: provider.revision,
            tools: provider.tools.clone()?,
        })
    }

    /// Replace effective source scopes and end activations that no longer match.
    pub fn reconcile(&self, allowed: BTreeMap<ControlScope, ApplicationContext>) {
        let mut state = self.state();
        state.allowed = allowed;
        let ended = state
            .active
            .iter()
            .filter(|(_, active)| state.allowed.get(&active.scope) != Some(&active.context))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for session in ended {
            state.end(session, DeactivationReason::FocusChanged);
        }
        self.publish(&state);
    }

    /// Start one real source session after its adapter admits input.
    pub fn activate(
        &self,
        scope: ControlScope,
        context: &ApplicationContext,
        capabilities: ControlCapabilities,
        selected: ControlId,
        selection: SelectionInput,
    ) -> Option<ActiveSession> {
        let mut state = self.state();
        let current = state.providers.get(&context.provider)?;
        if state.allowed.get(&scope) != Some(context)
            || current.connection != context.connection
            || current.revision != context.revision
            || !context.tools.tools().iter().any(|tool| tool.id == selected)
        {
            return None;
        }
        let old = state
            .active
            .iter()
            .filter(|(_, active)| active.scope == scope)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for session in old {
            state.end(session, DeactivationReason::CaptureEnded);
        }
        let source = if let Some(id) = state.sources.get(&scope.source) {
            id.clone()
        } else {
            let id = ControlId::new(format!("source-{}", state.next_id())).ok()?;
            state.sources.insert(scope.source.clone(), id.clone());
            id
        };
        let lease = ActiveSession {
            id: SessionId(state.next_id()),
            connection: context.connection,
            revision: context.revision,
        };
        state.active.insert(
            lease.id,
            Active {
                lease,
                scope,
                context: context.clone(),
                selected: selected.clone(),
                sequence: 0,
            },
        );
        let sent = state.emit(
            context.connection,
            ServerMessage::Activated(Activation {
                session: lease.id,
                context_revision: context.revision,
                source: ControlSource {
                    id: source,
                    capabilities,
                },
                selected,
                selection,
            }),
        );
        self.publish(&state);
        sent.then_some(lease)
    }

    /// End exactly this activation, leaving a newer incarnation untouched.
    pub fn end(&self, lease: ActiveSession) {
        let mut state = self.state();
        if state
            .active
            .get(&lease.id)
            .is_some_and(|active| active.lease == lease)
        {
            state.end(lease.id, DeactivationReason::CaptureEnded);
            self.publish(&state);
        }
    }

    /// Publish a host-owned tool selection in the same ordered queue as input.
    pub fn select(&self, lease: ActiveSession, tool: ControlId) {
        let mut state = self.state();
        let Some(active) = state
            .active
            .get_mut(&lease.id)
            .filter(|active| active.lease == lease)
        else {
            return;
        };
        if active.selected == tool
            || !active
                .context
                .tools
                .tools()
                .iter()
                .any(|candidate| candidate.id == tool)
        {
            return;
        }
        active.selected = tool.clone();
        state.emit(
            lease.connection,
            ServerMessage::Selected {
                session: lease.id,
                tool,
            },
        );
        self.publish(&state);
    }

    /// Route one real adapter input. No socket send runs in this method.
    pub fn input(&self, lease: ActiveSession, event: ControlInput) {
        let mut state = self.state();
        let Some(active) = state
            .active
            .get_mut(&lease.id)
            .filter(|active| active.lease == lease)
        else {
            return;
        };
        if matches!(&event, ControlInput::Rotation(rotation) if !rotation.amount.is_finite()) {
            return;
        }
        active.sequence += 1;
        let sequence = active.sequence;
        let tool = active.selected.clone();
        let time_us = u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX);
        state.emit(
            lease.connection,
            ServerMessage::Input {
                session: lease.id,
                first: sequence,
                last: sequence,
                time_us,
                tool,
                event,
            },
        );
        self.publish(&state);
    }

    /// Current validated values for a presenter, scoped to an exact activation.
    #[must_use]
    pub fn values(&self, lease: ActiveSession) -> BTreeMap<ControlId, ParameterState> {
        let state = self.state();
        let Some(active) = state
            .active
            .get(&lease.id)
            .filter(|active| active.lease == lease)
        else {
            return BTreeMap::new();
        };
        state
            .providers
            .get(&active.context.provider)
            .filter(|provider| {
                provider.connection == lease.connection && provider.revision == lease.revision
            })
            .map(|provider| provider.values.clone())
            .unwrap_or_default()
    }
}
