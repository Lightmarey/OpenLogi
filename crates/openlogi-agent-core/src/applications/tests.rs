use super::*;
use openlogi_sdk::ControlFeatures;
use openlogi_sdk::client::{Client, ClientError};
use openlogi_sdk::transport::{self, Endpoint};
use openlogi_sdk::{
    ClientMessage, ControlKind, ControlLabel, ParameterValue, Rotation, RotationPhase,
    RotationResolution, TactileMode, Tool,
};
use std::time::Duration;

impl ActiveSession {
    pub(crate) const fn id(self) -> SessionId {
        self.id
    }
}

pub(crate) fn provider(id: &str) -> Provider {
    Provider {
        id: ProviderId::new(id.into()).unwrap(),
        name: ControlLabel::new("Parameter example").unwrap(),
    }
}

pub(crate) fn tools(value: i32) -> ToolSet {
    ToolSet::new(vec![
        Tool {
            id: ControlId::new("zoom".into()).unwrap(),
            name: ControlLabel::new("Zoom").unwrap(),
            icon: openlogi_core::binding::ActionRingIcon::Gauge,
            tactile: TactileMode::Smooth,
            parameter: Some(ParameterState::Available(
                ParameterValue::new(value, 0, 100, format!("{value}%")).unwrap(),
            )),
        },
        Tool {
            id: ControlId::new("command".into()).unwrap(),
            name: ControlLabel::new("Next item").unwrap(),
            icon: openlogi_core::binding::ActionRingIcon::NextTab,
            tactile: TactileMode::Ratchet,
            parameter: None,
        },
    ])
    .unwrap()
}

pub(crate) fn capabilities() -> ControlCapabilities {
    // A plain encoder exercises the public host independently of Craft's adapter.
    ControlCapabilities {
        kind: ControlKind::Rotary,
        features: ControlFeatures::PRESS | ControlFeatures::DETENTS,
        resolution: RotationResolution::UNKNOWN,
    }
}

pub(crate) async fn next(client: &mut Client) -> ServerMessage {
    tokio::time::timeout(Duration::from_secs(2), client.next())
        .await
        .unwrap()
        .unwrap()
}

pub(crate) async fn context(
    host: &ApplicationHost,
    id: &ProviderId,
    revision: u64,
) -> ApplicationContext {
    let mut changed = host.subscribe();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(context) = host
                .resolve(id)
                .filter(|context| context.revision == revision)
            {
                return context;
            }
            changed.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}

fn activate(host: &ApplicationHost, context: &ApplicationContext) -> ActiveSession {
    let scope = ControlScope::new("private-device-key".into(), Some("example-editor".into()));
    host.reconcile([(scope.clone(), context.clone())].into());
    host.activate(
        scope,
        context,
        capabilities(),
        ControlId::new("zoom".into()).unwrap(),
        SelectionInput::TouchTap,
    )
    .unwrap()
}

fn motion(amount: f64) -> ControlInput {
    ControlInput::Rotation(Rotation {
        amount,
        raw: 2,
        detents: 0,
        resolution: RotationResolution::new(1, 4),
        tactile: TactileMode::Smooth,
        phase: RotationPhase::Move,
        pressed: false,
        touched: false,
        speed: 8,
    })
}

struct ExampleProcess(std::process::Child);

impl Drop for ExampleProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
#[ignore = "requires OPENLOGI_SDK_EXAMPLE pointing to the built parameters example"]
#[expect(
    clippy::too_many_lines,
    reason = "the real child process and its parameter, command and reconnect proofs share one lifetime"
)]
async fn sdk_parameter_example_completes_the_parameter_command_and_reconnection_loop() {
    use crate::crown::{CONTACT_LIFETIME, CrownDisplay};
    use crate::runtime::HidppSessionId;
    use openlogi_core::crown::{CrownFeedback, CrownProfile};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sdk.sock");
    let endpoint = Endpoint::at(path.clone());
    let state_file = directory.path().join("preview.json");
    let output = std::fs::File::create(directory.path().join("preview.log")).unwrap();
    let host = Arc::new(ApplicationHost::default());
    let server = ApplicationServer::start_at(Arc::clone(&host), &endpoint).unwrap();
    let binary =
        std::env::var_os("OPENLOGI_SDK_EXAMPLE").expect("build the parameters example first");
    let child = std::process::Command::new(binary)
        .arg(path)
        .arg(&state_file)
        .stdout(output)
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let process = ExampleProcess(child);
    let provider = ProviderId::new("example-preview".into()).unwrap();
    let first = context(&host, &provider, 1).await;
    let active = activate(&host, &first);
    let mut saved = CrownProfile::default();
    saved.set_enabled(true);
    let profile = super::crown::profile_for(&saved, &first).unwrap();
    let display = CrownDisplay::with_applications(Arc::clone(&host));
    let owner = HidppSessionId::new("craft");
    display.show_application(
        &owner,
        openlogi_ipc::CrownHud::new(&profile, profile.functions()[0].id().clone()),
        CONTACT_LIFETIME,
        Some(active),
    );
    let initial = display.observe(0).await;
    host.input(active, motion(1.0));
    let zoom = tokio::time::timeout(Duration::from_secs(2), display.observe(initial.generation))
        .await
        .unwrap();
    assert_eq!(
        zoom.hud.unwrap().functions[0].feedback,
        Some(CrownFeedback::Parameter(ParameterState::Available(
            ParameterValue::new(105, 25, 200, "105%".into()).unwrap()
        )))
    );
    assert!(
        std::fs::read_to_string(&state_file)
            .unwrap()
            .contains("\"zoom\":105")
    );
    host.select(active, ControlId::new("item".into()).unwrap());
    display.show_application(
        &owner,
        openlogi_ipc::CrownHud::new(&profile, profile.functions()[1].id().clone()),
        CONTACT_LIFETIME,
        Some(active),
    );
    let selected = display.observe(0).await;
    host.input(
        active,
        ControlInput::Press {
            down: true,
            reserved: false,
        },
    );
    host.input(
        active,
        ControlInput::Press {
            down: false,
            reserved: false,
        },
    );
    let item = tokio::time::timeout(Duration::from_secs(2), display.observe(selected.generation))
        .await
        .unwrap();
    assert_eq!(
        item.hud.unwrap().functions[1].feedback,
        Some(CrownFeedback::Parameter(ParameterState::Available(
            ParameterValue::new(1, 0, 2, "Portrait".into()).unwrap()
        )))
    );
    assert!(
        std::fs::read_to_string(&state_file)
            .unwrap()
            .contains("\"item\":1")
    );
    server.stop().await;
    assert!(host.values(active).is_empty());
    let replacement = ApplicationServer::start_at(Arc::clone(&host), &endpoint).unwrap();
    let second = context(&host, &provider, 1).await;
    assert_ne!(first.connection, second.connection);
    let resumed = activate(&host, &second);
    assert_eq!(
        host.values(resumed)[&ControlId::new("zoom".into()).unwrap()],
        ParameterState::Available(ParameterValue::new(105, 25, 200, "105%".into()).unwrap())
    );
    drop(process);
    let mut changes = host.subscribe();
    tokio::time::timeout(Duration::from_secs(2), async {
        while host.resolve(&provider).is_some() {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    replacement.stop().await;
    println!(
        "SDK EXAMPLE PASS: native process, real zoom, command state, parameter HUD, endpoint restart, reconnect, lease cleanup"
    );
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one live application lease proves input, feedback and reactivation ordering"
)]
async fn real_connection_routes_inputs_application_values_and_the_read_only_hud() {
    use crate::crown::{CONTACT_LIFETIME, CrownDisplay};
    use crate::runtime::HidppSessionId;
    use openlogi_core::crown::{CrownFeedback, CrownFunction, CrownFunctionId, CrownProfile};
    let directory = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::at(directory.path().join("sdk.sock"));
    let host = Arc::new(ApplicationHost::default());
    let _server = ApplicationServer::start_at(Arc::clone(&host), &endpoint).unwrap();
    let provider = provider("parameters");
    let mut client = Client::connect_at(&endpoint, provider.clone())
        .await
        .unwrap();
    client.set_context(1, Some(tools(50))).await.unwrap();
    let context = context(&host, &provider.id, 1).await;
    let active = activate(&host, &context);
    let ServerMessage::Activated(activation) = next(&mut client).await else {
        panic!("activation");
    };
    assert_eq!(activation.session, active.id);
    assert!(
        !activation
            .source
            .capabilities
            .features
            .contains(ControlFeatures::TOUCH)
    );
    assert!(!activation.source.id.as_ref().contains("private-device-key"));
    let profile = CrownProfile::new(
        true,
        context
            .tools
            .tools()
            .iter()
            .map(|tool| {
                CrownFunction::new(
                    CrownFunctionId::new(tool.id.as_ref().into()).unwrap(),
                    tool.name.as_ref(),
                    tool.icon,
                )
                .unwrap()
            })
            .collect(),
    )
    .unwrap();
    let display = CrownDisplay::with_applications(Arc::clone(&host));
    display.show_application(
        &HidppSessionId::new("craft"),
        openlogi_ipc::CrownHud::new(&profile, profile.functions()[0].id().clone()),
        CONTACT_LIFETIME,
        Some(active),
    );
    let shown = display.observe(0).await;
    host.input(active, motion(0.5));
    let ServerMessage::Input {
        session,
        first,
        last,
        time_us,
        tool,
        event: ControlInput::Rotation(rotation),
    } = next(&mut client).await
    else {
        panic!("input");
    };
    assert_eq!((session, first, last), (active.id(), 1, 1));
    assert!(time_us > 0);
    // This is the application's real state. Only its published result reaches HUD.
    let mut zoom = 50;
    if rotation.amount > 0.0 {
        zoom += 5;
    }
    let measured =
        ParameterState::Available(ParameterValue::new(zoom, 0, 100, format!("{zoom}%")).unwrap());
    client
        .set_parameter(session, activation.context_revision, tool, measured.clone())
        .await
        .unwrap();
    let changed = tokio::time::timeout(Duration::from_secs(2), display.observe(shown.generation))
        .await
        .unwrap();
    assert_eq!(
        changed.hud.unwrap().functions[0].feedback,
        Some(CrownFeedback::Parameter(measured))
    );
    host.select(active, ControlId::new("command".into()).unwrap());
    assert!(
        matches!(next(&mut client).await, ServerMessage::Selected { tool, .. } if tool.as_ref() == "command")
    );
    host.input(
        active,
        ControlInput::Press {
            down: true,
            reserved: false,
        },
    );
    assert!(
        matches!(next(&mut client).await, ServerMessage::Input { first: 2, tool, event: ControlInput::Press { down: true, reserved: false }, .. } if tool.as_ref() == "command")
    );
    host.end(active);
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Deactivated {
            reason: DeactivationReason::CaptureEnded,
            ..
        }
    ));
    let resumed = activate(&host, &context);
    next(&mut client).await;
    assert_eq!(
        host.values(resumed)[&ControlId::new("zoom".into()).unwrap()],
        ParameterState::Available(ParameterValue::new(55, 0, 100, "55%".into()).unwrap()),
        "a new activation reads the connection's latest real parameter"
    );
}

#[tokio::test]
async fn context_and_focus_changes_end_old_activations_and_reject_their_values() {
    let directory = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::at(directory.path().join("sdk.sock"));
    let host = Arc::new(ApplicationHost::default());
    let _server = ApplicationServer::start_at(Arc::clone(&host), &endpoint).unwrap();
    let provider = provider("parameters");
    let mut client = Client::connect_at(&endpoint, provider.clone())
        .await
        .unwrap();
    client.set_context(1, Some(tools(25))).await.unwrap();
    let first = context(&host, &provider.id, 1).await;
    let old = activate(&host, &first);
    next(&mut client).await;
    client.set_context(2, Some(tools(75))).await.unwrap();
    let second = context(&host, &provider.id, 2).await;
    assert!(
        matches!(next(&mut client).await, ServerMessage::Deactivated { session, reason: DeactivationReason::ContextReplaced } if session == old.id())
    );
    let current = activate(&host, &second);
    next(&mut client).await;
    client
        .set_parameter(
            old.id(),
            1,
            ControlId::new("zoom".into()).unwrap(),
            ParameterState::Unavailable,
        )
        .await
        .unwrap();
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Error {
            code: ErrorCode::StaleContext,
            ..
        }
    ));
    assert_eq!(
        host.values(current)[&ControlId::new("zoom".into()).unwrap()],
        tools(75).tools()[0].parameter.clone().unwrap()
    );
    host.reconcile(BTreeMap::new());
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Deactivated {
            reason: DeactivationReason::FocusChanged,
            ..
        }
    ));
    client
        .set_parameter(
            current.id(),
            2,
            ControlId::new("zoom".into()).unwrap(),
            ParameterState::Unavailable,
        )
        .await
        .unwrap();
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Error {
            code: ErrorCode::StaleSession,
            ..
        }
    ));
    host.input(old, motion(10.0));
    assert!(host.values(old).is_empty());
    client.set_context(1, Some(tools(0))).await.unwrap();
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Error {
            code: ErrorCode::StaleContext,
            ..
        }
    ));
    assert_eq!(host.resolve(&provider.id).unwrap(), second);
}

#[tokio::test]
async fn duplicate_providers_do_not_replace_each_other_and_reconnects_get_new_leases() {
    let directory = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::at(directory.path().join("sdk.sock"));
    let host = Arc::new(ApplicationHost::default());
    let _server = ApplicationServer::start_at(Arc::clone(&host), &endpoint).unwrap();
    let info = provider("parameters");
    let first = Client::connect_at(&endpoint, info.clone()).await.unwrap();
    let connection = first.connection();
    assert!(matches!(
        Client::connect_at(&endpoint, info.clone()).await,
        Err(ClientError::Rejected {
            code: ErrorCode::DuplicateProvider,
            ..
        })
    ));
    let _other = Client::connect_at(&endpoint, provider("other-provider"))
        .await
        .unwrap();
    let mut directory_changes = host.directory.subscribe();
    drop(first);
    tokio::time::timeout(Duration::from_secs(2), async {
        while directory_changes
            .borrow()
            .providers
            .iter()
            .any(|provider| provider.id == info.id)
        {
            directory_changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let second = Client::connect_at(&endpoint, info).await.unwrap();
    assert_ne!(connection, second.connection());
}

#[tokio::test]
async fn a_slow_real_client_loses_its_lease_and_old_cleanup_cannot_end_its_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::at(directory.path().join("sdk.sock"));
    let host = Arc::new(ApplicationHost::default());
    let _server = ApplicationServer::start_at(Arc::clone(&host), &endpoint).unwrap();
    let info = provider("parameters");
    let mut client = Client::connect_at(&endpoint, info.clone()).await.unwrap();
    client.set_context(1, Some(tools(50))).await.unwrap();
    let first = context(&host, &info.id, 1).await;
    let active = activate(&host, &first);
    next(&mut client).await;
    // No yield permits the socket writer to drain these real ordered edges.
    for index in 0..65 {
        host.input(
            active,
            ControlInput::Press {
                down: index % 2 == 0,
                reserved: false,
            },
        );
    }
    assert!(host.resolve(&info.id).is_none());
    assert!(host.values(active).is_empty());
    let mut replacement = Client::connect_at(&endpoint, info.clone()).await.unwrap();
    replacement.set_context(1, Some(tools(80))).await.unwrap();
    let current = context(&host, &info.id, 1).await;
    assert_ne!(current.connection, first.connection);
    drop(client);
    tokio::task::yield_now().await;
    assert_eq!(host.resolve(&info.id).unwrap(), current);
}

#[tokio::test]
async fn incompatible_or_oversized_connections_never_acquire_a_provider_lease() {
    use tokio::io::AsyncWriteExt as _;
    let directory = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::at(directory.path().join("sdk.sock"));
    let host = Arc::new(ApplicationHost::default());
    let _server = ApplicationServer::start_at(Arc::clone(&host), &endpoint).unwrap();
    let mut raw = endpoint.connect().await.unwrap();
    transport::send(
        &mut raw,
        &ClientMessage::Register {
            version: openlogi_sdk::VERSION + 1,
            provider: provider("bad-version"),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        transport::receive::<ServerMessage>(&mut raw).await.unwrap(),
        ServerMessage::Error {
            code: ErrorCode::Version,
            ..
        }
    ));
    let mut oversized = endpoint.connect().await.unwrap();
    oversized
        .get_mut()
        .write_all(&vec![b'x'; openlogi_sdk::MAX_FRAME_BYTES + 1])
        .await
        .unwrap();
    assert!(matches!(
        transport::receive::<ServerMessage>(&mut oversized)
            .await
            .unwrap(),
        ServerMessage::Error {
            code: ErrorCode::InvalidMessage,
            ..
        }
    ));
    assert!(host.directory.borrow().providers.is_empty());
}
