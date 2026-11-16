//! Device-independent application tools over OpenLogi's local JSON-lines protocol.

mod protocol;
pub use protocol::*;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod transport;
