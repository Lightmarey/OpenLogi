//! Terminal preview with real zoom and item selection, including reconnection.
//! Optional arguments are an isolated endpoint, state file and action limit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use openlogi_sdk::ToolIcon;
use openlogi_sdk::client::Client;
use openlogi_sdk::transport::Endpoint;
use openlogi_sdk::{
    ControlId, ControlInput, ControlLabel, ParameterState, ParameterValue, Provider, ProviderId,
    ServerMessage, SessionId, TactileMode, Tool, ToolSet,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(serde::Serialize)]
struct Preview {
    zoom: i32,
    item: usize,
    actions: usize,
    #[serde(skip)]
    zoom_fraction: f64,
    #[serde(skip)]
    item_fraction: f64,
}

const ITEMS: [&str; 3] = ["Landscape", "Portrait", "Still life"];

impl Default for Preview {
    fn default() -> Self {
        Self {
            zoom: 100,
            item: 0,
            actions: 0,
            zoom_fraction: 0.0,
            item_fraction: 0.0,
        }
    }
}

impl Preview {
    fn value(&self, tool: &str) -> Result<ParameterState> {
        let value = match tool {
            "zoom" => ParameterValue::new(self.zoom, 25, 200, format!("{}%", self.zoom))?,
            "item" => {
                ParameterValue::new(i32::try_from(self.item)?, 0, 2, ITEMS[self.item].into())?
            }
            _ => return Err("unknown preview tool".into()),
        };
        Ok(ParameterState::Available(value))
    }

    fn tools(&self) -> Result<ToolSet> {
        Ok(ToolSet::new(vec![
            Tool {
                id: ControlId::new("zoom".into())?,
                name: ControlLabel::new("Preview zoom")?,
                icon: ToolIcon::Gauge,
                tactile: TactileMode::Smooth,
                parameter: Some(self.value("zoom")?),
            },
            Tool {
                id: ControlId::new("item".into())?,
                name: ControlLabel::new("Preview item")?,
                icon: ToolIcon::NewTab,
                tactile: TactileMode::Ratchet,
                parameter: Some(self.value("item")?),
            },
        ])?)
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "zoom is clamped to 25–200 and item offset is reduced modulo three before conversion"
    )]
    fn input(&mut self, tool: &str, event: &ControlInput) -> bool {
        let old = (self.zoom, self.item);
        match (tool, event) {
            ("zoom", ControlInput::Rotation(rotation)) if rotation.amount.is_finite() => {
                let requested = (f64::from(self.zoom) + self.zoom_fraction + rotation.amount * 5.0)
                    .clamp(25.0, 200.0);
                self.zoom = requested.round() as i32;
                self.zoom_fraction = requested - f64::from(self.zoom);
            }
            ("item", ControlInput::Rotation(rotation)) if rotation.amount.is_finite() => {
                let requested = self.item_fraction + rotation.amount;
                let steps = requested.trunc();
                self.item_fraction = requested - steps;
                let offset = steps.rem_euclid(3.0) as usize;
                self.item = (self.item + offset) % ITEMS.len();
            }
            (
                "item",
                ControlInput::Press {
                    down: false,
                    reserved: false,
                },
            ) => {
                self.item = (self.item + 1) % ITEMS.len();
            }
            _ => {}
        }
        if old == (self.zoom, self.item) {
            return false;
        }
        self.actions += 1;
        true
    }

    fn render(&self, file: Option<&Path>) -> Result<()> {
        let width = usize::try_from(self.zoom / 10)?;
        println!(
            "{}  {}%  {}",
            ITEMS[self.item],
            self.zoom,
            "▰".repeat(width)
        );
        if let Some(file) = file {
            std::fs::write(file, serde_json::to_vec(self)?)?;
        }
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let endpoint = match arguments.first() {
        Some(path) => Endpoint::at(PathBuf::from(path)),
        None => Endpoint::current()?,
    };
    let state_file = arguments.get(1).map(PathBuf::from);
    let action_limit = arguments
        .get(2)
        .map(|value| value.to_string_lossy().parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    let provider = Provider {
        id: ProviderId::new("example-preview".into())?,
        name: ControlLabel::new("OpenLogi preview example")?,
    };
    let mut preview = Preview::default();
    preview.render(state_file.as_deref())?;
    loop {
        let mut client = match Client::connect_at(&endpoint, provider.clone()).await {
            Ok(client) => client,
            Err(error) => {
                eprintln!("Waiting for OpenLogi ({error})");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if let Err(error) = client.set_context(1, Some(preview.tools()?)).await {
            eprintln!("OpenLogi disconnected ({error})");
            continue;
        }
        let mut sessions = BTreeMap::<SessionId, u64>::new();
        'connected: loop {
            let message = match client.next().await {
                Ok(message) => message,
                Err(error) => {
                    eprintln!("OpenLogi disconnected ({error})");
                    break;
                }
            };
            match message {
                ServerMessage::Activated(activation) => {
                    sessions.insert(activation.session, activation.context_revision);
                    for tool in preview.tools()?.tools() {
                        if let Err(error) = client
                            .set_parameter(
                                activation.session,
                                activation.context_revision,
                                tool.id.clone(),
                                preview.value(tool.id.as_ref())?,
                            )
                            .await
                        {
                            eprintln!("OpenLogi disconnected ({error})");
                            break 'connected;
                        }
                    }
                }
                ServerMessage::Deactivated { session, .. } => {
                    sessions.remove(&session);
                }
                ServerMessage::Input {
                    session,
                    tool,
                    event,
                    ..
                } if sessions.contains_key(&session) => {
                    if preview.input(tool.as_ref(), &event) {
                        preview.render(state_file.as_deref())?;
                        let value = preview.value(tool.as_ref())?;
                        for (&session, &revision) in &sessions {
                            if let Err(error) = client
                                .set_parameter(session, revision, tool.clone(), value.clone())
                                .await
                            {
                                eprintln!("OpenLogi disconnected ({error})");
                                break 'connected;
                            }
                        }
                        if action_limit != 0 && preview.actions >= action_limit {
                            return Ok(());
                        }
                    }
                }
                ServerMessage::Error { code, message } => eprintln!("OpenLogi {code:?}: {message}"),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlogi_sdk::{Rotation, RotationPhase, RotationResolution};

    #[test]
    fn preview_changes_real_state_and_publishes_its_actual_values() {
        let mut preview = Preview::default();
        let rotate = |amount| {
            ControlInput::Rotation(Rotation {
                amount,
                raw: 1,
                detents: 0,
                resolution: RotationResolution::UNKNOWN,
                tactile: TactileMode::Smooth,
                phase: RotationPhase::Move,
                pressed: false,
                touched: false,
                speed: 1,
            })
        };
        assert!(preview.input("zoom", &rotate(0.5)));
        assert_eq!(preview.zoom, 103);
        assert!(preview.input("zoom", &rotate(1000.0)));
        assert_eq!(preview.zoom, 200);
        assert!(preview.input(
            "item",
            &ControlInput::Press {
                down: false,
                reserved: false
            }
        ));
        assert_eq!(preview.item, 1);
        assert_eq!(
            preview.value("item").unwrap(),
            ParameterState::Available(ParameterValue::new(1, 0, 2, "Portrait".into()).unwrap())
        );
        assert!(!preview.input(
            "item",
            &ControlInput::Press {
                down: false,
                reserved: true
            }
        ));
    }
}
