//! Recoverable, coalescing connection to noninteractive GNOME Shell actors.

use std::time::Duration;

use futures_lite::StreamExt as _;
use gpui::AssetSource as _;
use openlogi_ipc::CrownHud;
use serde::Serialize;
use tokio::sync::watch;
use tracing::warn;
use zbus::{Connection, Proxy, connection::Builder};

const SERVICE: &str = "org.openlogi.CrownHud";
const PATH: &str = "/org/openlogi/CrownHud";
const BRIDGE_VERSION: u32 = 1;
const RETRY_PERIOD: Duration = Duration::from_secs(1);

pub(super) struct Presenter {
    state: watch::Sender<Option<CrownHud>>,
}

impl Presenter {
    pub(super) fn start() -> std::io::Result<Self> {
        let (state, updates) = watch::channel(None);
        openlogi_core::worker::spawn("openlogi-crown-shell", move |runtime| {
            runtime.block_on(present(updates));
        })?;
        Ok(Self { state })
    }

    pub(super) fn show(&self, hud: Option<CrownHud>) {
        self.state.send_replace(hud);
    }
}

/// Each successful connection owns one Shell lease. Replacing or dropping it
/// releases the unique bus name, so an old lease cannot dismiss a new actor.
struct Bridge {
    connection: Connection,
    owner: String,
}

impl Bridge {
    async fn connect() -> anyhow::Result<Self> {
        let connection = Builder::session()?
            .method_timeout(RETRY_PERIOD)
            .build()
            .await?;
        let owner = service_owner(&connection).await?;
        let bridge = Self { connection, owner };
        bridge.check().await?;
        Ok(bridge)
    }

    async fn proxy(&self) -> zbus::Result<Proxy<'_>> {
        Proxy::new(&self.connection, self.owner.as_str(), PATH, SERVICE).await
    }

    async fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            service_owner(&self.connection).await? == self.owner,
            "Crown HUD extension restarted"
        );
        let version: u32 = self.proxy().await?.call("GetVersion", &()).await?;
        anyhow::ensure!(
            version == BRIDGE_VERSION,
            "Crown HUD extension version {version} differs from {BRIDGE_VERSION}"
        );
        Ok(())
    }

    async fn show(&self, hud: Option<&CrownHud>) -> anyhow::Result<()> {
        self.check().await?;
        let proxy = self.proxy().await?;
        if let Some(hud) = hud {
            proxy.call::<_, _, ()>("Show", &(encode(hud)?,)).await?;
        } else {
            proxy.call::<_, _, ()>("Hide", &()).await?;
        }
        Ok(())
    }
}

async fn service_owner(connection: &Connection) -> zbus::Result<String> {
    Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await?
    .call("GetNameOwner", &(SERVICE,))
    .await
}

async fn present(mut updates: watch::Receiver<Option<CrownHud>>) {
    let mut failure = None;
    while updates.has_changed().is_ok() {
        match serve_connection(&mut updates).await {
            Ok(()) => return,
            Err(error) => report_failure(&mut failure, &error),
        }
        // Unavailable and incompatible extensions retry on one bounded cadence.
        // Watch retains the latest snapshot while this connection is down.
        tokio::time::sleep(RETRY_PERIOD).await;
    }
}

async fn serve_connection(updates: &mut watch::Receiver<Option<CrownHud>>) -> anyhow::Result<()> {
    let bridge = Bridge::connect().await?;
    let proxy = Proxy::new(&bridge.connection, SERVICE, PATH, SERVICE).await?;
    let mut owners = proxy.receive_owner_changed().await?;
    // Subscribe before checking ownership again. A quick disable/enable in
    // the same Shell process must invalidate the old lease too.
    let hud = updates.borrow_and_update().clone();
    bridge.show(hud.as_ref()).await?;
    loop {
        tokio::select! {
            changed = updates.changed() => {
                if changed.is_err() { return Ok(()); }
                let hud = updates.borrow_and_update().clone();
                bridge.show(hud.as_ref()).await?;
            }
            _ = owners.next() => anyhow::bail!("Crown HUD extension ownership changed"),
        }
    }
}

fn report_failure(previous: &mut Option<String>, error: &anyhow::Error) {
    let message = error.to_string();
    if previous.as_ref() != Some(&message) {
        warn!(%error, "Crown HUD bridge unavailable; waiting for the extension");
        *previous = Some(message);
    }
}

#[derive(Serialize)]
struct Function<'a> {
    id: &'a str,
    name: &'a str,
    svg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    feedback: Option<Feedback>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Feedback {
    SystemVolume,
}

#[derive(Serialize)]
struct Presentation<'a> {
    selected: &'a str,
    functions: Vec<Function<'a>>,
    accent: String,
}

fn css(color: gpui::Hsla) -> String {
    let color: gpui::Rgba = color.into();
    format!(
        "rgba({:.0},{:.0},{:.0},{:.3})",
        color.r * 255.,
        color.g * 255.,
        color.b * 255.,
        color.a
    )
}

fn encode(hud: &CrownHud) -> anyhow::Result<String> {
    let colors = super::theme::CrownColors::for_appearance(gpui::WindowAppearance::Dark);
    let functions = hud
        .functions
        .iter()
        .map(|function| {
            let bytes = openlogi_ui::action_icons::ActionIcons
                .load(function.icon.asset_path())?
                .ok_or_else(|| {
                    anyhow::anyhow!("missing Crown icon {}", function.icon.asset_path())
                })?;
            Ok(Function {
                id: function.id.as_ref(),
                name: &function.name,
                svg: std::str::from_utf8(&bytes)?.to_owned(),
                feedback: function.feedback.as_ref().map(|feedback| match feedback {
                    openlogi_core::crown::CrownFeedback::SystemVolume => Feedback::SystemVolume,
                }),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(serde_json::to_string(&Presentation {
        selected: hud.selected.as_ref(),
        functions,
        accent: css(colors.selected),
    })?)
}

#[cfg(test)]
mod tests;
