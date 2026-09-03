//! Type-safe UUIDv7 identity types for the Choruz full-pipeline ID system.
//!
//! Every domain concept that needs a durable, globally-unique identity gets its
//! own newtype wrapper around `uuid::Uuid`.  This prevents accidental misuse
//! (e.g. passing a `TurnId` where a `CommandId` is expected) while keeping
//! serialisation / display / parsing uniform.
//!
//! # ID hierarchy
//!
//! ```text
//! MessageId (inbound)
//!   └── RouteId (routing decision)
//!        └── CommandId (agent command)
//!             └── TurnId (logical reply, stable across retries)
//!                  ├── AttemptId #1
//!                  │    ├── ToolCallId #1
//!                  │    └── ToolCallId #2
//!                  ├── AttemptId #2
//!                  └── ReplyEventId (conversation commit)
//!                       └── DeliveryId (fanout to sinks)
//! ```

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Macro: define a newtype wrapper around Uuid with standard trait impls
// ---------------------------------------------------------------------------

macro_rules! define_id {
    (
        $(#[$meta:meta])*
        $name:ident
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generate a new ID using UUIDv7 (time-ordered, random tail).
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wrap an existing [`Uuid`] value.
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Return the inner [`Uuid`].
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Return the inner [`Uuid`] by value.
            #[must_use]
            pub const fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl AsRef<Uuid> for $name {
            fn as_ref(&self) -> &Uuid {
                &self.0
            }
        }
    };
}

// ---------------------------------------------------------------------------
// ID types — one per domain concept
// ---------------------------------------------------------------------------

define_id! {
    /// Client-generated message ID used for inbound retry dedup.
    ClientMsgId
}

define_id! {
    /// Server-assigned inbound message ID (unique per conversation event).
    MessageId
}

define_id! {
    /// Unique ID for a single routing decision (message x agent).
    RouteId
}

define_id! {
    /// Unique ID for an agent command dispatched by the router.
    CommandId
}

define_id! {
    /// Logical reply ID — stable across retries of the same command.
    TurnId
}

define_id! {
    /// Unique ID for one execution attempt of a command.
    AttemptId
}

define_id! {
    /// Unique ID for one external tool invocation within an attempt.
    ToolCallId
}

define_id! {
    /// Unique ID for one fanout delivery to an external sink.
    DeliveryId
}

define_id! {
    /// Unique ID for the reply event committed to the conversation stream.
    ReplyEventId
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `::new()` produces a valid UUIDv7.
    #[test]
    fn new_ids_are_uuid_v7() {
        let mid = MessageId::new();
        assert_eq!(mid.as_uuid().get_version(), Some(uuid::Version::SortRand));
    }

    /// Verify that two consecutive IDs are monotonically ordered.
    #[test]
    fn ids_are_monotonic() {
        let a = MessageId::new();
        let b = MessageId::new();
        // UUIDv7 encodes timestamp in the high bits, so string comparison works.
        assert!(a.to_string() <= b.to_string());
    }

    /// Round-trip through Display / FromStr.
    #[test]
    fn display_fromstr_roundtrip() {
        let original = CommandId::new();
        let text = original.to_string();
        let parsed: CommandId = text.parse().expect("parse failed");
        assert_eq!(original, parsed);
    }

    /// Round-trip through serde_json.
    #[test]
    fn serde_json_roundtrip() {
        let original = TurnId::new();
        let json = serde_json::to_string(&original).expect("serialize failed");
        // Should be a plain JSON string (no wrapper object).
        assert!(json.starts_with('"'));
        let parsed: TurnId = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(original, parsed);
    }

    /// Ensure different ID types cannot be accidentally mixed (compile-time
    /// guarantee — this test just documents intent).
    #[test]
    fn type_safety_documented() {
        fn takes_turn(_id: TurnId) {}
        fn takes_attempt(_id: AttemptId) {}

        let t = TurnId::new();
        let a = AttemptId::new();

        takes_turn(t);
        takes_attempt(a);
        // The following would NOT compile:
        // takes_turn(a);
        // takes_attempt(t);
    }

    /// Debug format includes the type name.
    #[test]
    fn debug_includes_type_name() {
        let id = DeliveryId::new();
        let dbg = format!("{id:?}");
        assert!(dbg.starts_with("DeliveryId("));
    }

    /// FromStr rejects garbage.
    #[test]
    fn fromstr_rejects_garbage() {
        assert!("not-a-uuid".parse::<MessageId>().is_err());
    }

    /// From<Uuid> conversion works.
    #[test]
    fn from_uuid_conversion() {
        let raw = Uuid::now_v7();
        let id = RouteId::from(raw);
        assert_eq!(*id.as_uuid(), raw);
        let back: Uuid = id.into();
        assert_eq!(back, raw);
    }

    // -----------------------------------------------------------------------
    // Edge-case tests
    // -----------------------------------------------------------------------

    /// Empty string is rejected by FromStr.
    #[test]
    fn fromstr_rejects_empty_string() {
        assert!("".parse::<MessageId>().is_err());
    }

    /// Uppercase UUID string is accepted (UUID parsing is case-insensitive).
    #[test]
    fn fromstr_accepts_uppercase() {
        let id = TurnId::new();
        let upper = id.to_string().to_uppercase();
        let parsed: TurnId = upper.parse().expect("uppercase parse failed");
        assert_eq!(id, parsed);
    }

    /// Non-hyphenated (compact) UUID format is accepted by FromStr.
    #[test]
    fn fromstr_accepts_non_hyphenated() {
        let id = AttemptId::new();
        let compact = id.to_string().replace('-', "");
        let parsed: AttemptId = compact.parse().expect("compact parse failed");
        assert_eq!(id, parsed);
    }

    /// Hash is consistent: same ID always hashes the same way.
    #[test]
    fn hash_is_consistent() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let id = CommandId::new();
        let mut h1 = DefaultHasher::new();
        id.hash(&mut h1);
        let hash1 = h1.finish();

        let mut h2 = DefaultHasher::new();
        id.hash(&mut h2);
        let hash2 = h2.finish();

        assert_eq!(hash1, hash2);
    }

    /// IDs work as HashMap keys (exercises Hash + Eq).
    #[test]
    fn id_as_hashmap_key() {
        use std::collections::HashMap;

        let mut map = HashMap::new();
        let id1 = ToolCallId::new();
        let id2 = ToolCallId::new();
        map.insert(id1, "first");
        map.insert(id2, "second");
        assert_eq!(map.len(), 2);
        assert_eq!(map[&id1], "first");
        assert_eq!(map[&id2], "second");
    }

    /// Default produces a valid UUIDv7 (same as ::new()).
    #[test]
    fn default_produces_valid_uuid() {
        let id = ReplyEventId::default();
        assert_eq!(id.as_uuid().get_version(), Some(uuid::Version::SortRand));
    }

    /// All 9 ID types produce distinct values (no shared state across types).
    #[test]
    fn all_id_types_produce_distinct_values() {
        let ids: Vec<String> = vec![
            ClientMsgId::new().to_string(),
            MessageId::new().to_string(),
            RouteId::new().to_string(),
            CommandId::new().to_string(),
            TurnId::new().to_string(),
            AttemptId::new().to_string(),
            ToolCallId::new().to_string(),
            DeliveryId::new().to_string(),
            ReplyEventId::new().to_string(),
        ];
        // All 9 should be unique.
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), 9);
    }

    /// Copy semantics: original is still usable after copy.
    #[test]
    fn copy_semantics_work() {
        let original = MessageId::new();
        let copied = original; // Copy, not move
        assert_eq!(original, copied);
        // Both are usable after copy:
        let _ = original.to_string();
        let _ = copied.to_string();
    }

    /// Clone produces an equal value.
    #[test]
    fn clone_produces_equal_value() {
        let original = DeliveryId::new();
        #[allow(clippy::clone_on_copy)]
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    /// Serde: ID inside a struct is serialized as a flat string field.
    #[test]
    fn serde_in_struct_is_flat_string() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            id: MessageId,
            label: String,
        }

        let w = Wrapper {
            id: MessageId::new(),
            label: "test".into(),
        };
        let json = serde_json::to_value(&w).expect("serialize");
        // The "id" field should be a string, not an object.
        assert!(json["id"].is_string());
        let parsed: Wrapper = serde_json::from_value(json.clone()).expect("deserialize");
        assert_eq!(parsed.id, w.id);
    }

    /// Serde: ID as a HashMap key serializes as string keys in JSON.
    #[test]
    fn serde_as_hashmap_key() {
        use std::collections::HashMap;

        let mut map = HashMap::new();
        let id = TurnId::new();
        map.insert(id, 42);

        let json = serde_json::to_string(&map).expect("serialize");
        // JSON should have string keys, e.g. {"<uuid>":42}
        assert!(json.contains(':'));
        let parsed: HashMap<TurnId, i32> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed[&id], 42);
    }

    /// AsRef<Uuid> trait works correctly.
    #[test]
    fn as_ref_uuid_works() {
        let id = ClientMsgId::new();
        let uuid_ref: &Uuid = id.as_ref();
        assert_eq!(uuid_ref, id.as_uuid());
    }
}
