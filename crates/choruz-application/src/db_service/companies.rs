use choruz_common::{AppError, new_id, now};
use choruz_domain::{Company, CompanyMember, Principal, PrincipalType};

use super::DbService;
use super::helpers::{row_to_company, row_to_company_member, row_to_principal};

impl DbService {
    pub(crate) async fn principal_can_access_workspace(
        &self,
        principal: &Principal,
        workspace_id: &str,
    ) -> Result<bool, AppError> {
        let client = self.store.connect().await?;
        if principal.workspace_id == workspace_id {
            let row = client
                .query_opt(
                    "SELECT deleted_at FROM company WHERE id = $1",
                    &[&workspace_id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("workspace company access check: {e}")))?;
            return Ok(row
                .map(|row| {
                    row.get::<_, Option<chrono::DateTime<chrono::Utc>>>("deleted_at")
                        .is_none()
                })
                .unwrap_or(true));
        }

        let row = client
            .query_opt(
                "SELECT 1
                 FROM company_member cm
                 JOIN company c ON c.id = cm.company_id
                 WHERE cm.company_id = $1
                   AND cm.principal_id = $2
                   AND c.deleted_at IS NULL",
                &[&workspace_id, &principal.id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("company workspace access check: {e}")))?;

        Ok(row.is_some())
    }

    // ── Company reads (Phase 1C) ──────────────────────────────────────

    /// List companies that a principal belongs to (via company_member join).
    ///
    /// Also includes any company whose `id` equals the principal's `workspace_id`,
    /// mirroring the in-memory `ChatApp::list_companies` semantics.
    /// Excludes soft-deleted companies (`deleted_at IS NOT NULL`) by default.
    /// Includes archived companies (with `archived_at` populated) so they can
    /// be shown greyed-out in the UI.
    pub async fn list_companies(&self, principal_id: &str) -> Result<Vec<Company>, AppError> {
        let principal = self.get_principal(principal_id).await?;
        let client = self.store.connect().await?;

        let rows = client
            .query(
                "SELECT DISTINCT c.id, c.name, c.slug, c.description, c.avatar_url,
                        c.owner_id, c.agents_active, c.folder_path,
                        c.multi_harness_accounts, c.archived_at, c.deleted_at,
                        c.created_at, c.updated_at
                 FROM company c
                 LEFT JOIN company_member cm ON cm.company_id = c.id AND cm.principal_id = $1
                 WHERE (cm.principal_id IS NOT NULL OR c.id = $2)
                   AND c.deleted_at IS NULL
                 ORDER BY c.name",
                &[&principal_id, &principal.workspace_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_companies: {e}")))?;

        Ok(rows.iter().map(row_to_company).collect())
    }

    /// Get a single company by ID from the database.
    pub async fn get_company(&self, company_id: &str) -> Result<Company, AppError> {
        let client = self.store.connect().await?;
        let row = client
            .query_opt(
                "SELECT id, name, slug, description, avatar_url, owner_id,
                        agents_active, folder_path, multi_harness_accounts,
                        archived_at, deleted_at, created_at, updated_at
                 FROM company WHERE id = $1",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("get_company: {e}")))?;

        let row = row.ok_or_else(|| AppError::NotFound(format!("company {company_id}")))?;
        Ok(row_to_company(&row))
    }

    /// List members of a company.
    pub async fn list_company_members(
        &self,
        company_id: &str,
    ) -> Result<Vec<CompanyMember>, AppError> {
        // Verify company exists first
        let client = self.store.connect().await?;
        let exists = client
            .query_opt(
                "SELECT id FROM company WHERE id = $1 AND deleted_at IS NULL",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_company_members check: {e}")))?;
        if exists.is_none() {
            return Err(AppError::NotFound(format!("company {company_id}")));
        }

        let rows = client
            .query(
                "SELECT principal_id, joined_at
                 FROM company_member
                 WHERE company_id = $1",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_company_members: {e}")))?;

        Ok(rows.iter().map(row_to_company_member).collect())
    }

    /// List agents belonging to a company (workspace).
    ///
    /// Mirrors `ChatApp::list_agents_for_company`: filters principals where
    /// `workspace_id = company_id`, `type = 'agent'`, and not disabled.
    pub async fn list_agents_for_company(
        &self,
        company_id: &str,
    ) -> Result<Vec<Principal>, AppError> {
        let client = self.store.connect().await?;
        let rows = client
            .query(
                "SELECT id, workspace_id, type, name, avatar_url, secret_hash,
                        channel_visibility, disabled, deleted_at, created_at, updated_at
                 FROM principal
                 WHERE workspace_id = $1 AND type = 'agent'
                   AND disabled = false AND deleted_at IS NULL
                 ORDER BY name",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("list_agents_for_company: {e}")))?;

        Ok(rows.iter().map(row_to_principal).collect())
    }

    // ── Company writes (Phase 2C) ──────────────────────────────────────

    /// Create a company in the database with the creator as owner.
    ///
    /// Mirrors `ChatApp::create_company` validation: only humans can
    /// create, slug must be unique for the owner.
    pub async fn create_company(
        &self,
        request: crate::CreateCompanyRequest,
    ) -> Result<Company, AppError> {
        let actor = self.get_principal(&request.actor_id).await?;
        if !matches!(actor.principal_type, PrincipalType::Human) {
            return Err(AppError::Forbidden(
                "only humans can create companies".into(),
            ));
        }

        let slug = request.slug.unwrap_or_else(|| {
            request
                .name
                .to_lowercase()
                .replace(|c: char| !c.is_alphanumeric(), "-")
        });

        let client = self.store.connect().await?;

        // Slugs identify a company within one account. Different accounts may
        // use the same familiar slug without sharing any workspace data.
        let existing = client
            .query_opt(
                "SELECT id FROM company
                 WHERE owner_id = $1 AND slug = $2 AND deleted_at IS NULL",
                &[&request.actor_id, &slug],
            )
            .await
            .map_err(|e| AppError::Internal(format!("create_company slug check: {e}")))?;
        if existing.is_some() {
            return Err(AppError::Conflict(format!(
                "company slug '{slug}' already exists for this account"
            )));
        }

        let id = new_id();
        let timestamp = now();
        let desc = request.description.as_deref().unwrap_or("");
        let folder = request.folder_path.as_deref().unwrap_or("");

        client
            .execute(
                "INSERT INTO company (id, name, slug, description, avatar_url, owner_id, agents_active, folder_path, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, '', $5, true, $6, $7, $8)",
                &[&id, &request.name, &slug, &desc, &request.actor_id, &folder, &timestamp, &timestamp],
            )
            .await
            .map_err(|e| AppError::Internal(format!("create_company insert: {e}")))?;

        // Add the creator as a member; ownership lives on company.owner_id.
        client
            .execute(
                "INSERT INTO company_member (company_id, principal_id, joined_at)
                 VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING",
                &[&id, &request.actor_id, &timestamp],
            )
            .await
            .map_err(|e| AppError::Internal(format!("create_company insert owner: {e}")))?;

        self.record_audit(
            &actor.workspace_id,
            &actor.id,
            "company.created",
            "company",
            &id,
            serde_json::json!({"name": &request.name, "slug": &slug}),
        )
        .await?;

        Ok(Company {
            id,
            name: request.name,
            slug,
            description: request.description,
            avatar_url: None,
            owner_id: request.actor_id,
            agents_active: true,
            folder_path: request.folder_path,
            multi_harness_accounts: false,
            archived_at: None,
            deleted_at: None,
            created_at: timestamp,
            updated_at: timestamp,
        })
    }

    /// Update a company in the database.
    ///
    /// Every company member can update the company.
    pub async fn update_company(
        &self,
        company_id: &str,
        request: crate::UpdateCompanyRequest,
    ) -> Result<Company, AppError> {
        let actor = self.get_principal(&request.actor_id).await?;

        let client = self.store.connect().await?;

        // Check company exists
        let _company = client
            .query_opt(
                "SELECT id FROM company WHERE id = $1 AND deleted_at IS NULL",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("update_company check: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("company {company_id}")))?;

        // Membership is the permission boundary.
        let member_row = client
            .query_opt(
                "SELECT 1 FROM company_member WHERE company_id = $1 AND principal_id = $2",
                &[&company_id, &request.actor_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("update_company member check: {e}")))?;
        if member_row.is_none() {
            return Err(AppError::Forbidden("not a company member".into()));
        }

        let timestamp = now();

        // Update fields individually (same pattern as update_group)
        if let Some(ref name) = request.name {
            client
                .execute(
                    "UPDATE company SET name = $1, updated_at = $2 WHERE id = $3",
                    &[name, &timestamp, &company_id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("update_company name: {e}")))?;
        }
        if let Some(ref desc) = request.description {
            client
                .execute(
                    "UPDATE company SET description = $1, updated_at = $2 WHERE id = $3",
                    &[desc, &timestamp, &company_id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("update_company description: {e}")))?;
        }
        if let Some(ref avatar) = request.avatar_url {
            client
                .execute(
                    "UPDATE company SET avatar_url = $1, updated_at = $2 WHERE id = $3",
                    &[avatar, &timestamp, &company_id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("update_company avatar: {e}")))?;
        }
        if let Some(active) = request.agents_active {
            client
                .execute(
                    "UPDATE company SET agents_active = $1, updated_at = $2 WHERE id = $3",
                    &[&active, &timestamp, &company_id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("update_company agents_active: {e}")))?;
        }
        if let Some(ref fp) = request.folder_path {
            let folder: Option<&str> = if fp.is_empty() {
                None
            } else {
                Some(fp.as_str())
            };
            client
                .execute(
                    "UPDATE company SET folder_path = $1, updated_at = $2 WHERE id = $3",
                    &[&folder, &timestamp, &company_id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("update_company folder_path: {e}")))?;
        }
        if let Some(enabled) = request.multi_harness_accounts {
            client
                .execute(
                    "UPDATE company SET multi_harness_accounts = $1, updated_at = $2 WHERE id = $3",
                    &[&enabled, &timestamp, &company_id],
                )
                .await
                .map_err(|e| {
                    AppError::Internal(format!("update_company multi_harness_accounts: {e}"))
                })?;
        }
        // If no fields provided, still bump updated_at
        if request.name.is_none()
            && request.description.is_none()
            && request.avatar_url.is_none()
            && request.agents_active.is_none()
            && request.folder_path.is_none()
            && request.multi_harness_accounts.is_none()
        {
            client
                .execute(
                    "UPDATE company SET updated_at = $1 WHERE id = $2",
                    &[&timestamp, &company_id],
                )
                .await
                .map_err(|e| AppError::Internal(format!("update_company timestamp: {e}")))?;
        }

        self.record_audit(
            &actor.workspace_id,
            &actor.id,
            "company.updated",
            "company",
            company_id,
            serde_json::json!({
                "name": request.name,
                "description": request.description,
                "avatar_url": request.avatar_url,
                "agents_active": request.agents_active,
                "folder_path": request.folder_path,
                "multi_harness_accounts": request.multi_harness_accounts,
            }),
        )
        .await?;

        // Fetch and return updated company
        self.get_company(company_id).await
    }

    /// Soft-delete a company (set `deleted_at = NOW()`).
    ///
    /// Only the owner can delete. The company is hidden from listings but
    /// data remains intact for 30-day recovery.
    pub async fn delete_company(&self, company_id: &str, actor_id: &str) -> Result<(), AppError> {
        let actor = self.get_principal(actor_id).await?;
        let client = self.store.connect().await?;

        let row = client
            .query_opt(
                "SELECT owner_id, deleted_at FROM company WHERE id = $1",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("delete_company check: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("company {company_id}")))?;

        let owner_id: String = row.get("owner_id");
        if owner_id != actor_id {
            return Err(AppError::Forbidden("only company owner can delete".into()));
        }
        if row
            .get::<_, Option<chrono::DateTime<chrono::Utc>>>("deleted_at")
            .is_some()
        {
            return Ok(());
        }

        let timestamp = now();
        client
            .execute(
                "UPDATE company SET deleted_at = $1, updated_at = $1 WHERE id = $2",
                &[&timestamp, &company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("delete_company soft: {e}")))?;

        self.record_audit(
            &actor.workspace_id,
            &actor.id,
            "company.deleted",
            "company",
            company_id,
            serde_json::json!({}),
        )
        .await?;

        Ok(())
    }

    /// Archive a company (set `archived_at = NOW()`).
    ///
    /// Archived companies are read-only and appear greyed out in the UI.
    /// Every company member can archive.
    pub async fn archive_company(
        &self,
        company_id: &str,
        actor_id: &str,
    ) -> Result<Company, AppError> {
        let actor = self.get_principal(actor_id).await?;
        let client = self.store.connect().await?;

        // Check company exists
        let _company = client
            .query_opt(
                "SELECT id FROM company WHERE id = $1 AND deleted_at IS NULL",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("archive_company check: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("company {company_id}")))?;

        // Membership is the permission boundary.
        let member_row = client
            .query_opt(
                "SELECT 1 FROM company_member WHERE company_id = $1 AND principal_id = $2",
                &[&company_id, &actor_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("archive_company member check: {e}")))?;
        if member_row.is_none() {
            return Err(AppError::Forbidden("not a company member".into()));
        }

        let timestamp = now();
        client
            .execute(
                "UPDATE company SET archived_at = $1, updated_at = $1 WHERE id = $2",
                &[&timestamp, &company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("archive_company: {e}")))?;

        self.record_audit(
            &actor.workspace_id,
            &actor.id,
            "company.archived",
            "company",
            company_id,
            serde_json::json!({}),
        )
        .await?;

        self.get_company(company_id).await
    }

    /// Unarchive a company (set `archived_at = NULL`).
    ///
    /// Every company member can unarchive.
    pub async fn unarchive_company(
        &self,
        company_id: &str,
        actor_id: &str,
    ) -> Result<Company, AppError> {
        let actor = self.get_principal(actor_id).await?;
        let client = self.store.connect().await?;

        // Check company exists
        let _company = client
            .query_opt(
                "SELECT id FROM company WHERE id = $1 AND deleted_at IS NULL",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("unarchive_company check: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("company {company_id}")))?;

        // Membership is the permission boundary.
        let member_row = client
            .query_opt(
                "SELECT 1 FROM company_member WHERE company_id = $1 AND principal_id = $2",
                &[&company_id, &actor_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("unarchive_company member check: {e}")))?;
        if member_row.is_none() {
            return Err(AppError::Forbidden("not a company member".into()));
        }

        let timestamp = now();
        let archived_at: Option<chrono::DateTime<chrono::Utc>> = None;
        client
            .execute(
                "UPDATE company SET archived_at = $1, updated_at = $2 WHERE id = $3",
                &[&archived_at, &timestamp, &company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("unarchive_company: {e}")))?;

        self.record_audit(
            &actor.workspace_id,
            &actor.id,
            "company.unarchived",
            "company",
            company_id,
            serde_json::json!({}),
        )
        .await?;

        self.get_company(company_id).await
    }

    /// Add a member to a company in the database.
    ///
    /// Every company member can add another member.
    pub async fn add_company_member(
        &self,
        company_id: &str,
        request: crate::AddCompanyMemberRequest,
    ) -> Result<CompanyMember, AppError> {
        let actor = self.get_principal(&request.actor_id).await?;
        let target = self.get_principal(&request.principal_id).await?;

        let client = self.store.connect().await?;

        // Check company exists
        let _company = client
            .query_opt(
                "SELECT id FROM company WHERE id = $1 AND deleted_at IS NULL",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("add_company_member check: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("company {company_id}")))?;
        drop(client);

        if !self
            .principal_can_access_workspace(&target, company_id)
            .await?
        {
            return Err(AppError::Forbidden("cross-workspace access denied".into()));
        }

        let client = self.store.connect().await?;

        // Membership is the permission boundary.
        let actor_member = client
            .query_opt(
                "SELECT 1 FROM company_member WHERE company_id = $1 AND principal_id = $2",
                &[&company_id, &request.actor_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("add_company_member actor check: {e}")))?;
        if actor_member.is_none() {
            return Err(AppError::Forbidden("not a company member".into()));
        }
        let timestamp = now();

        client
            .execute(
                "INSERT INTO company_member (company_id, principal_id, joined_at)
                 VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING",
                &[&company_id, &request.principal_id, &timestamp],
            )
            .await
            .map_err(|e| AppError::Internal(format!("add_company_member insert: {e}")))?;

        self.record_audit(
            &actor.workspace_id,
            &actor.id,
            "company.member_added",
            "company",
            company_id,
            serde_json::json!({"principal_id": &request.principal_id}),
        )
        .await?;

        Ok(CompanyMember {
            principal_id: request.principal_id,
            joined_at: timestamp,
        })
    }

    /// Remove a member from a company in the database.
    ///
    /// Every company member can remove another member, except the company owner.
    pub async fn remove_company_member(
        &self,
        company_id: &str,
        actor_id: &str,
        target_id: &str,
    ) -> Result<(), AppError> {
        let actor = self.get_principal(actor_id).await?;
        let client = self.store.connect().await?;

        // Check company exists
        let company = client
            .query_opt(
                "SELECT id, owner_id FROM company WHERE id = $1 AND deleted_at IS NULL",
                &[&company_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove_company_member check: {e}")))?
            .ok_or_else(|| AppError::NotFound(format!("company {company_id}")))?;

        // Membership is the permission boundary.
        let actor_member = client
            .query_opt(
                "SELECT 1 FROM company_member WHERE company_id = $1 AND principal_id = $2",
                &[&company_id, &actor_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove_company_member actor check: {e}")))?;
        if actor_member.is_none() {
            return Err(AppError::Forbidden("not a company member".into()));
        }
        let owner_id: String = company.get("owner_id");
        if target_id == owner_id {
            return Err(AppError::Forbidden("cannot remove company owner".into()));
        }

        client
            .execute(
                "DELETE FROM company_member WHERE company_id = $1 AND principal_id = $2",
                &[&company_id, &target_id],
            )
            .await
            .map_err(|e| AppError::Internal(format!("remove_company_member delete: {e}")))?;

        self.record_audit(
            &actor.workspace_id,
            &actor.id,
            "company.member_removed",
            "company",
            company_id,
            serde_json::json!({"principal_id": target_id}),
        )
        .await?;

        Ok(())
    }
}
