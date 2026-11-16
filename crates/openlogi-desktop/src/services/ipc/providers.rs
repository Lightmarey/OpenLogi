//! Read-only provider directory, observed on a connection-scoped IPC ledger.

use openlogi_ipc::{
    ClientKind,
    client::{self, Observer},
};
use tokio::sync::mpsc;

use super::{GuiUpdate, RECONNECT_DELAY};

pub(super) async fn observe(updates: &mpsc::UnboundedSender<GuiUpdate>) {
    loop {
        if let Ok(client) = client::connect_as(ClientKind::Gui).await {
            let mut observer = Observer::application_providers(client);
            loop {
                match observer.next().await {
                    Ok(Some(directory)) => {
                        if updates
                            .send(GuiUpdate::ApplicationProviders(directory.providers))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        }
        if updates
            .send(GuiUpdate::ApplicationProviders(Vec::new()))
            .is_err()
        {
            return;
        }
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}
