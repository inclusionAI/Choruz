//! CLI configuration and native session discovery, without a platform database.
//! Callers own process execution, permissions and persisted binding updates.

pub mod binding;
pub mod computer_use;
pub mod headless;
pub mod session_catalog;

pub use binding::{
    AuditActor, BindingState, CodexTerminalCaptureInput, CodexTerminalCaptureMetadata,
    CreateBindingInput, DriverType, RuntimeBinding, TerminalSessionAnchor,
    TerminalSessionAnchorInput, TriggerType, latest_native_session, normalize_workspace_path,
};
pub use session_catalog::{
    HarnessKind, NativeSessionSummary, SessionAccount, SessionCatalogScanner, SessionScanResult,
};
