//! Bounded internal collaborators. The final executor keeps its existing identity
//! and permissions; this configuration cannot provision accounts or devices.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    Serial,
    Parallel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub name: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Team {
    pub order: Order,
    pub members: Vec<Member>,
}

impl Team {
    pub fn reviewer(prompt: String) -> Self {
        Self {
            order: Order::Serial,
            members: vec![Member {
                name: "reviewer".into(),
                prompt,
            }],
        }
    }

    pub fn validate(&self, max_agents: usize) -> Result<(), String> {
        let mut names = BTreeSet::new();
        if !(1..=4).contains(&max_agents) || self.members.len() + 1 > max_agents {
            return Err("Execution team exceeds the total-agent limit".into());
        }
        for member in &self.members {
            if member.name.is_empty()
                || member.name.len() > 40
                || !member
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || !names.insert(&member.name)
                || member.prompt.trim().is_empty()
                || member.prompt.len() > 4000
            {
                return Err(
                    "Team members require unique simple names and prompts within 4000 bytes".into(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn total_limit_includes_executor_and_rejects_permission_fields() {
        let mut team = Team::reviewer("Check evidence".into());
        assert!(team.validate(1).is_err());
        assert!(team.validate(2).is_ok());
        team.members.push(team.members[0].clone());
        assert!(team.validate(4).is_err());
        assert!(
            serde_json::from_str::<Team>(r#"{"order":"serial","members":[],"account":"other"}"#)
                .is_err()
        );
    }
}
