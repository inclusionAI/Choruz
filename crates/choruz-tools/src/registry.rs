//! Tool Registry: tracks which tools are mutating vs read-only and their
//! mutation policy.
//!
//! Read-only tools skip the idempotency check entirely. Mutating tools are
//! further classified by [`MutationPolicy`] to determine replay behaviour.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Mutation policy
// ---------------------------------------------------------------------------

/// How the Tool Gateway handles repeated invocations of a mutating tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationPolicy {
    /// The external API accepts an idempotency key; safe to retry with the
    /// same key.
    IdempotentApi,
    /// Cache the first result and return it on subsequent calls with the
    /// same `tool_call_id`. Never re-execute.
    CacheAndSuppress,
    /// Requires operator review before execution (future extension).
    OperatorReview,
}

// ---------------------------------------------------------------------------
// Tool descriptor
// ---------------------------------------------------------------------------

/// Static metadata about a registered tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Machine-readable tool name (e.g. `"github_comment"`).
    pub name: String,
    /// Whether this tool performs external mutations.
    pub is_mutating: bool,
    /// Policy for handling repeated calls (only meaningful when `is_mutating`).
    pub mutation_policy: MutationPolicy,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Registry of known tools and their mutation semantics.
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolDescriptor>,
}

impl ToolRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool.  Overwrites any previous registration with the same name.
    pub fn register(&mut self, descriptor: ToolDescriptor) {
        self.tools.insert(descriptor.name.clone(), descriptor);
    }

    /// Register a read-only tool (convenience helper).
    pub fn register_read_only(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.tools.insert(
            name.clone(),
            ToolDescriptor {
                name,
                is_mutating: false,
                mutation_policy: MutationPolicy::CacheAndSuppress, // irrelevant for read-only
            },
        );
    }

    /// Register a mutating tool with a specific policy (convenience helper).
    pub fn register_mutating(&mut self, name: impl Into<String>, policy: MutationPolicy) {
        let name = name.into();
        self.tools.insert(
            name.clone(),
            ToolDescriptor {
                name,
                is_mutating: true,
                mutation_policy: policy,
            },
        );
    }

    /// Look up a tool by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ToolDescriptor> {
        self.tools.get(name)
    }

    /// Returns `true` if the named tool is mutating.
    ///
    /// Unknown tools are treated as mutating (safe default).
    #[must_use]
    pub fn is_mutating(&self, name: &str) -> bool {
        self.tools.get(name).is_none_or(|desc| desc.is_mutating)
    }

    /// Returns the mutation policy for the named tool.
    ///
    /// Unknown tools default to [`MutationPolicy::CacheAndSuppress`].
    #[must_use]
    pub fn mutation_policy(&self, name: &str) -> MutationPolicy {
        self.tools
            .get(name)
            .map_or(MutationPolicy::CacheAndSuppress, |desc| {
                desc.mutation_policy
            })
    }

    /// Return an iterator over all registered tool names.
    pub fn tool_names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }

    /// Number of registered tools.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a registry pre-populated with a standard set of tools.
///
/// This is a reasonable starting point; callers can extend or override.
#[must_use]
pub fn default_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();

    // Read-only tools
    reg.register_read_only("file_read");
    reg.register_read_only("search_code");
    reg.register_read_only("list_files");
    reg.register_read_only("git_log");
    reg.register_read_only("git_diff");
    reg.register_read_only("git_status");

    // Mutating tools -- idempotent API
    reg.register_mutating("github_comment", MutationPolicy::IdempotentApi);
    reg.register_mutating("github_pr_create", MutationPolicy::IdempotentApi);

    // Mutating tools -- cache and suppress
    reg.register_mutating("file_write", MutationPolicy::CacheAndSuppress);
    reg.register_mutating("git_commit", MutationPolicy::CacheAndSuppress);
    reg.register_mutating("git_push", MutationPolicy::CacheAndSuppress);
    reg.register_mutating("shell_exec", MutationPolicy::CacheAndSuppress);
    reg.register_mutating("jira_create", MutationPolicy::CacheAndSuppress);
    reg.register_mutating("webhook_post", MutationPolicy::CacheAndSuppress);

    reg
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry() {
        let reg = ToolRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.get("unknown").is_none());
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = ToolRegistry::new();
        reg.register_read_only("file_read");
        reg.register_mutating("git_push", MutationPolicy::CacheAndSuppress);

        assert_eq!(reg.len(), 2);
        assert!(!reg.is_empty());

        let fr = reg.get("file_read").unwrap();
        assert!(!fr.is_mutating);

        let gp = reg.get("git_push").unwrap();
        assert!(gp.is_mutating);
        assert_eq!(gp.mutation_policy, MutationPolicy::CacheAndSuppress);
    }

    #[test]
    fn unknown_tool_is_mutating_by_default() {
        let reg = ToolRegistry::new();
        assert!(reg.is_mutating("totally_unknown_tool"));
        assert_eq!(
            reg.mutation_policy("totally_unknown_tool"),
            MutationPolicy::CacheAndSuppress,
        );
    }

    #[test]
    fn read_only_tool_not_mutating() {
        let mut reg = ToolRegistry::new();
        reg.register_read_only("search_code");
        assert!(!reg.is_mutating("search_code"));
    }

    #[test]
    fn default_registry_has_tools() {
        let reg = default_registry();
        assert!(!reg.is_empty());
        assert!(!reg.is_mutating("file_read"));
        assert!(reg.is_mutating("git_push"));
    }

    #[test]
    fn mutation_policy_serde() {
        let json = serde_json::to_string(&MutationPolicy::IdempotentApi).unwrap();
        assert_eq!(json, "\"idempotent_api\"");
        let parsed: MutationPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, MutationPolicy::IdempotentApi);
    }
}
