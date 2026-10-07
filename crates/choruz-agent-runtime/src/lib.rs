//! CLI configuration and native session discovery, without a platform database.
//! Callers own process execution, permissions and persisted binding updates.

pub mod binding;
pub mod computer_use;
pub mod executable;
pub mod headless;
pub mod process;
pub mod process_scope;
pub mod session_catalog;
pub mod session_files;

pub use binding::{
    AuditActor, BindingState, CodexTerminalCaptureInput, CodexTerminalCaptureMetadata,
    CreateBindingInput, DriverType, RuntimeBinding, TerminalSessionAnchor,
    TerminalSessionAnchorInput, TriggerType, latest_native_session, normalize_workspace_path,
};
pub use session_catalog::{
    HarnessKind, NativeSessionSummary, SessionAccount, SessionCatalogScanner, SessionScanResult,
};
