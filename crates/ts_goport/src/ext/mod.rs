//! Not part of the typescript-go port.
//!
//! The port calls [`precheck::apply`] after parse and
//! [`precheck::emits_javascript`] when deciding emit.

/// The content-mapper identity of a file this extension rewrote. The emit
/// decision and the rewrite both compare against it.
pub const MAPPER_NAME: &str = "typegpu";

pub mod align;
pub mod overload;
pub mod precheck;
pub mod tsover;
pub mod typegpu;
