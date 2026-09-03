use choruz_common::{AppError, AppResult};
use choruz_domain::{Company, CompanyMember};

use crate::ChatApp;

impl ChatApp {
    /// List all companies with their members, without principal-based filtering.
    /// Used internally for DB backfill on startup.
    pub fn list_all_companies_internal(&self) -> Vec<(Company, Vec<CompanyMember>)> {
        let state = self.inner.read().expect("lock poisoned");
        state
            .companies
            .values()
            .map(|c| {
                let members = state
                    .company_members
                    .get(&c.id)
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                (c.clone(), members)
            })
            .collect()
    }

    /// Inject a company directly into memory, bypassing validation.
    /// Used by callers that need to mirror a database company in memory.
    pub fn inject_company(&self, company: Company, members: Vec<CompanyMember>) {
        let mut state = self.inner.write().expect("lock poisoned");
        let id = company.id.clone();
        if !state.companies.contains_key(&id) {
            state.companies.insert(id.clone(), company);
        }
        let entry = state.company_members.entry(id).or_default();
        for member in members {
            entry.entry(member.principal_id.clone()).or_insert(member);
        }
    }

    pub fn list_companies(&self, principal_id: &str) -> AppResult<Vec<Company>> {
        let state = self.inner.read().expect("lock poisoned");
        let mut result = Vec::new();
        for (company_id, members) in &state.company_members {
            if members.contains_key(principal_id)
                && let Some(company) = state.companies.get(company_id)
                && company.deleted_at.is_none()
            {
                result.push(company.clone());
            }
        }
        // Also include companies where the principal's workspace_id matches
        if let Some(ws) = state
            .principals
            .get(principal_id)
            .map(|p| p.workspace_id.as_str())
            && let Some(company) = state.companies.get(ws)
            && company.deleted_at.is_none()
            && !result.iter().any(|c| c.id == company.id)
        {
            result.push(company.clone());
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    pub fn get_company(&self, company_id: &str) -> AppResult<Company> {
        let state = self.inner.read().expect("lock poisoned");
        state
            .companies
            .get(company_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("company {company_id}")))
    }
}

#[cfg(test)]
mod companies_tests {
    use super::*;
    use crate::ChatApp;
    use choruz_domain::{Principal, PrincipalType};
    use chrono::Utc;

    fn mk_company(id: &str, name: &str) -> Company {
        Company {
            id: id.into(),
            name: name.into(),
            slug: id.to_lowercase(),
            description: None,
            avatar_url: None,
            owner_id: "owner".into(),
            agents_active: true,
            folder_path: None,
            multi_harness_accounts: false,
            archived_at: None,
            deleted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn mk_deleted_company(id: &str, name: &str) -> Company {
        Company {
            deleted_at: Some(Utc::now()),
            ..mk_company(id, name)
        }
    }

    fn mk_member(id: &str) -> CompanyMember {
        CompanyMember {
            principal_id: id.into(),
            joined_at: Utc::now(),
        }
    }

    fn mk_principal(id: &str, ws: &str) -> Principal {
        Principal {
            id: id.into(),
            workspace_id: ws.into(),
            principal_type: PrincipalType::Human,
            name: id.into(),
            avatar_url: None,
            scopes: vec![],
            secret_hash: None,
            disabled: false,
            deleted_at: None,
            channel_visibility: choruz_domain::ChannelVisibility::Visible,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            user_id: None,
        }
    }

    // inject_company ------------------------------------------------------

    #[test]
    fn inject_company_adds_company_and_members() {
        let app = ChatApp::new();
        app.inject_company(mk_company("c1", "Acme"), vec![mk_member("alice")]);
        let companies = app.list_all_companies_internal();
        assert_eq!(companies.len(), 1);
        assert_eq!(companies[0].0.name, "Acme");
        assert_eq!(companies[0].1.len(), 1);
        assert_eq!(companies[0].1[0].principal_id, "alice");
    }

    #[test]
    fn inject_company_is_idempotent_on_id_collision() {
        let app = ChatApp::new();
        app.inject_company(mk_company("c1", "Acme"), vec![]);
        app.inject_company(mk_company("c1", "DIFFERENT NAME"), vec![]);
        let companies = app.list_all_companies_internal();
        assert_eq!(companies.len(), 1);
        // Original name preserved (idempotent — second call ignored)
        assert_eq!(companies[0].0.name, "Acme");
    }

    #[test]
    fn inject_company_merges_new_members_into_existing_company() {
        let app = ChatApp::new();
        app.inject_company(mk_company("c1", "Acme"), vec![mk_member("alice")]);
        app.inject_company(mk_company("c1", "Acme"), vec![mk_member("bob")]);
        let companies = app.list_all_companies_internal();
        let members: Vec<&str> = companies[0]
            .1
            .iter()
            .map(|m| m.principal_id.as_str())
            .collect();
        assert!(members.contains(&"alice"));
        assert!(members.contains(&"bob"));
    }

    // get_company ---------------------------------------------------------

    #[test]
    fn get_company_returns_existing() {
        let app = ChatApp::new();
        app.inject_company(mk_company("c1", "Acme"), vec![]);
        assert_eq!(app.get_company("c1").unwrap().name, "Acme");
    }

    #[test]
    fn get_company_returns_not_found_for_unknown_id() {
        let app = ChatApp::new();
        let err = app.get_company("missing").unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    // list_companies ------------------------------------------------------

    #[test]
    fn list_companies_returns_only_companies_where_principal_is_member() {
        let app = ChatApp::new();
        app.inject_company(mk_company("c1", "Acme"), vec![mk_member("alice")]);
        app.inject_company(mk_company("c2", "BetaCorp"), vec![mk_member("bob")]);
        let alice_view = app.list_companies("alice").unwrap();
        assert_eq!(alice_view.len(), 1);
        assert_eq!(alice_view[0].id, "c1");
    }

    #[test]
    fn list_companies_results_are_sorted_by_name() {
        let app = ChatApp::new();
        app.inject_company(mk_company("z", "Zebra"), vec![mk_member("alice")]);
        app.inject_company(mk_company("a", "Apple"), vec![mk_member("alice")]);
        app.inject_company(mk_company("m", "Mango"), vec![mk_member("alice")]);
        let names: Vec<String> = app
            .list_companies("alice")
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, vec!["Apple", "Mango", "Zebra"]);
    }

    #[test]
    fn list_companies_includes_company_matching_principals_workspace_id() {
        let app = ChatApp::new();
        // Inject a principal whose workspace_id matches a company id but isn't a member.
        app.inject_principal(mk_principal("alice", "ws-x"));
        app.inject_company(mk_company("ws-x", "MyCo"), vec![]);
        let result = app.list_companies("alice").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "ws-x");
    }

    #[test]
    fn list_companies_excludes_deleted_companies() {
        let app = ChatApp::new();
        app.inject_principal(mk_principal("alice", "ws-x"));
        app.inject_company(
            mk_deleted_company("ws-x", "DeletedCo"),
            vec![mk_member("alice")],
        );

        let result = app.list_companies("alice").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn list_companies_does_not_duplicate_when_principal_is_member_and_workspace_matches() {
        let app = ChatApp::new();
        app.inject_principal(mk_principal("alice", "ws-x"));
        app.inject_company(mk_company("ws-x", "MyCo"), vec![mk_member("alice")]);
        let result = app.list_companies("alice").unwrap();
        assert_eq!(result.len(), 1);
    }
}
