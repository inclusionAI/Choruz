//! Evidence preparation and public exchange share the learning policy's owner
//! and revision lifecycle. Private source linkage never enters the public cache.
use super::DbService;
use choruz_common::AppError;
use choruz_community::behavior::{BehaviorRecord, COMMUNITY_REPOSITORY, CommunitySettings, counts};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub struct BehaviorClaim {
    pub id: String,
    pub token: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub binding_id: String,
    pub analyst_binding_id: String,
    pub problem_key: String,
    pub problem_id: String,
    pub description: String,
    pub episode_ref: String,
    pub references: Value,
    pub cursor: Value,
    pub applied_revision_id: Option<String>,
    pub generation: i64,
    pub solution: Option<choruz_community::behavior::SolutionVersion>,
    pub kind: choruz_community::behavior::EvidenceKind,
    pub occurrence_id: String,
}

pub struct PublicationClaim {
    pub id: String,
    pub token: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub binding_id: String,
    pub analyst_binding_id: String,
    pub generation: i64,
    pub record: BehaviorRecord,
}

impl DbService {
    pub async fn community_cached_records(
        &self,
    ) -> Result<BTreeMap<String, (String, BehaviorRecord)>, AppError> {
        let rows=self.store.connect().await?.query("SELECT record_id,blob_oid,payload FROM experience_community_record WHERE repository=$1", &[&COMMUNITY_REPOSITORY]).await.map_err(storage)?;
        rows.iter()
            .map(|row| {
                Ok((
                    row.get("record_id"),
                    (
                        row.get("blob_oid"),
                        serde_json::from_value(row.get("payload")).map_err(|_| {
                            AppError::Internal("Invalid cached behavior record".into())
                        })?,
                    ),
                ))
            })
            .collect()
    }
    pub async fn behavior_candidates(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        problem: &str,
    ) -> Result<Vec<Value>, AppError> {
        let query = problem
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| s.len() > 3)
            .take(24)
            .collect::<Vec<_>>()
            .join(" OR ");
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let client = self.store.connect().await?;
        let rows = client.query("WITH q AS (SELECT websearch_to_tsquery('english',$4) AS terms), candidates AS (
            SELECT e.payload,'local'::text AS origin,NULL::text AS revision,ts_rank_cd(to_tsvector('english',e.payload::text),q.terms) AS rank FROM experience_behavior_event e JOIN experience_policy p ON p.binding_id=e.binding_id AND p.workspace_id=e.workspace_id CROSS JOIN q WHERE p.workspace_id=$1 AND p.owner_id=$2 AND e.binding_id=$3 AND e.payload->'solution' IS NOT NULL AND e.payload->'solution'!='null'::jsonb AND to_tsvector('english',e.payload::text)@@q.terms
            UNION ALL
            SELECT c.payload,'community',c.revision,ts_rank_cd(to_tsvector('english',c.payload::text),q.terms) FROM experience_community_record c CROSS JOIN q WHERE c.repository=$5 AND c.payload->'solution' IS NOT NULL AND c.payload->'solution'!='null'::jsonb AND to_tsvector('english',c.payload::text)@@q.terms AND EXISTS(SELECT 1 FROM experience_policy WHERE workspace_id=$1 AND owner_id=$2 AND binding_id=$3 AND enabled AND community_settings->>'search'='true' AND community_settings->>'automatic_trial'='true')
            ) SELECT payload,origin,revision FROM candidates ORDER BY (origin='local') DESC,rank DESC,payload->>'id' LIMIT 8", &[&workspace,&owner,&binding,&query,&COMMUNITY_REPOSITORY]).await.map_err(storage)?;
        Ok(rows.iter().map(|r|json!({"record":r.get::<_,Value>("payload"),"origin":r.get::<_,String>("origin"),"revision":r.get::<_,Option<String>>("revision")})).collect())
    }

    pub async fn community_sync_due(&self) -> Result<bool, AppError> {
        let client = self.store.connect().await?;
        Ok(client.query_one("SELECT EXISTS(SELECT 1 FROM experience_policy WHERE enabled AND (community_settings->>'search'='true' OR community_settings->>'contribute'='true')) AND NOT EXISTS(SELECT 1 FROM experience_community_sync WHERE repository=$1 AND checked_at>NOW()-INTERVAL '5 minutes')",&[&COMMUNITY_REPOSITORY]).await.map_err(storage)?.get(0))
    }

    pub async fn community_snapshot_revision(&self) -> Result<Option<String>, AppError> {
        let client = self.store.connect().await?;
        Ok(client
            .query_opt(
                "SELECT revision FROM experience_community_sync WHERE repository=$1",
                &[&COMMUNITY_REPOSITORY],
            )
            .await
            .map_err(storage)?
            .and_then(|r| r.get(0)))
    }

    /// Replace the accepted cache atomically; changed immutable record contents
    /// reject the entire snapshot without losing the preceding accepted cache.
    pub async fn save_community_snapshot(
        &self,
        revision: &str,
        records: &[BehaviorRecord],
        objects: &BTreeMap<String, String>,
    ) -> Result<(), AppError> {
        for record in records {
            record.validate().map_err(AppError::Validation)?;
        }
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let mut ids = Vec::new();
        for record in records {
            ids.push(record.id.clone());
            let payload = json!(record);
            if let Some(existing) = tx.query_opt("SELECT payload FROM experience_community_record WHERE repository=$1 AND record_id=$2", &[&COMMUNITY_REPOSITORY,&record.id]).await.map_err(storage)?
                && existing.get::<_,Value>(0) != payload {
                return Err(AppError::Validation("Community changed an immutable evidence record".into()));
            }
            let oid = objects.get(&record.id).ok_or_else(|| {
                AppError::Validation("Community record lacks its source object".into())
            })?;
            tx.execute("INSERT INTO experience_community_record(repository,record_id,revision,payload,blob_oid) VALUES($1,$2,$3,$4,$5) ON CONFLICT(repository,record_id) DO UPDATE SET revision=EXCLUDED.revision,blob_oid=EXCLUDED.blob_oid,updated_at=NOW()", &[&COMMUNITY_REPOSITORY,&record.id,&revision,&payload,&oid]).await.map_err(storage)?;
        }
        tx.execute("DELETE FROM experience_community_record WHERE repository=$1 AND NOT(record_id=ANY($2))", &[&COMMUNITY_REPOSITORY,&ids]).await.map_err(storage)?;
        tx.execute("UPDATE experience_behavior_event e SET publication_state='accepted',updated_at=NOW() FROM experience_community_record c WHERE c.repository=$1 AND e.id=c.record_id AND e.public_payload=c.payload AND e.publication_state IN ('pending','uncertain')", &[&COMMUNITY_REPOSITORY]).await.map_err(storage)?;
        tx.execute("INSERT INTO experience_community_sync(repository,revision,checked_at,last_error) VALUES($1,$2,NOW(),NULL) ON CONFLICT(repository) DO UPDATE SET revision=EXCLUDED.revision,checked_at=NOW(),last_error=NULL", &[&COMMUNITY_REPOSITORY,&revision]).await.map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    pub async fn community_sync_error(&self, error: &str) -> Result<(), AppError> {
        self.store.connect().await?.execute("INSERT INTO experience_community_sync(repository,checked_at,last_error) VALUES($1,NOW(),$2) ON CONFLICT(repository) DO UPDATE SET checked_at=NOW(),last_error=EXCLUDED.last_error", &[&COMMUNITY_REPOSITORY,&error]).await.map_err(storage)?;
        Ok(())
    }

    pub async fn community_sync_unchanged(&self, revision: &str) -> Result<(), AppError> {
        self.store.connect().await?.execute("UPDATE experience_community_sync SET checked_at=NOW(),last_error=NULL WHERE repository=$1 AND revision=$2", &[&COMMUNITY_REPOSITORY,&revision]).await.map_err(storage)?;
        Ok(())
    }

    /// Lease one opted-in local record. An expired dispatched lease becomes
    /// uncertain, not retryable, because the remote PR may already exist.
    pub async fn claim_behavior_publication(&self) -> Result<Option<PublicationClaim>, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        tx.execute("UPDATE experience_behavior_event SET publication_state='uncertain',publication_error='Publication was interrupted; automatic retry is disabled to avoid duplicate contributions.' WHERE publication_state='publishing' AND lease_until<NOW()", &[]).await.map_err(storage)?;
        let row = tx.query_opt("SELECT e.id,e.workspace_id,e.binding_id,e.payload,p.owner_id,p.analyst_binding_id,p.generation FROM experience_behavior_event e JOIN experience_policy p ON p.binding_id=e.binding_id AND p.workspace_id=e.workspace_id WHERE p.enabled AND p.community_settings->>'contribute'='true' AND e.payload IS NOT NULL AND e.publication_state='local' AND e.publication_error IS NULL AND (e.lease_until IS NULL OR e.lease_until<NOW()) ORDER BY e.created_at FOR UPDATE OF e SKIP LOCKED LIMIT 1", &[]).await.map_err(storage)?;
        let Some(row) = row else {
            tx.commit().await.map_err(storage)?;
            return Ok(None);
        };
        let token = choruz_common::new_id();
        let id: String = row.get("id");
        tx.execute("UPDATE experience_behavior_event SET lease_token=$2,lease_until=NOW()+INTERVAL '10 minutes' WHERE id=$1", &[&id,&token]).await.map_err(storage)?;
        let claim = PublicationClaim {
            id,
            token,
            workspace_id: row.get("workspace_id"),
            binding_id: row.get("binding_id"),
            owner_id: row.get("owner_id"),
            analyst_binding_id: row.get("analyst_binding_id"),
            generation: row.get("generation"),
            record: serde_json::from_value(row.get("payload"))
                .map_err(|_| AppError::Internal("Invalid stored behavior evidence".into()))?,
        };
        tx.commit().await.map_err(storage)?;
        Ok(Some(claim))
    }

    /// Commit the reviewed projection before network dispatch. False means the
    /// lease or consent changed; callers must not publish that stale projection.
    pub async fn begin_behavior_publication(
        &self,
        claim: &PublicationClaim,
        public: &BehaviorRecord,
        review: &Value,
    ) -> Result<bool, AppError> {
        public.validate().map_err(AppError::Validation)?;
        if !claim.record.same_evidence_identity(public) || review["accepted"] != true {
            return Err(AppError::Validation(
                "Public contribution requires identity-preserving privacy review".into(),
            ));
        }
        let client = self.store.connect().await?;
        Ok(client.execute("UPDATE experience_behavior_event e SET public_payload=$4,privacy_review=$5,publication_state='publishing',updated_at=NOW() FROM experience_policy p WHERE e.id=$1 AND e.workspace_id=$2 AND e.lease_token=$3 AND e.lease_until>NOW() AND e.publication_state='local' AND p.binding_id=e.binding_id AND p.workspace_id=e.workspace_id AND p.owner_id=$6 AND p.generation=$7 AND p.enabled AND p.community_settings->>'contribute'='true'", &[&claim.id,&claim.workspace_id,&claim.token,&json!(public),&review,&claim.owner_id,&claim.generation]).await.map_err(storage)?!=0)
    }

    pub async fn finish_behavior_publication(
        &self,
        claim: &PublicationClaim,
        url: Option<&str>,
        dispatched: bool,
        error: Option<&str>,
    ) -> Result<(), AppError> {
        let status = if url.is_some() {
            "pending"
        } else if dispatched {
            "uncertain"
        } else {
            "blocked"
        };
        self.store.connect().await?.execute("UPDATE experience_behavior_event SET publication_state=$4,publication_url=$5,publication_error=$6,lease_token=NULL,lease_until=NULL,updated_at=NOW() WHERE id=$1 AND workspace_id=$2 AND lease_token=$3", &[&claim.id,&claim.workspace_id,&claim.token,&status,&url,&error]).await.map_err(storage)?;
        Ok(())
    }

    pub async fn behavior_feedback(
        &self,
        claim: &BehaviorClaim,
        references: &[String],
    ) -> Result<Vec<Value>, AppError> {
        let ids: Vec<_> = references
            .iter()
            .filter_map(|r| r.strip_prefix("message:"))
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let client = self.store.connect().await?;
        let rows = client.query("SELECT e.event_id,e.conversation_id,e.seq,e.content FROM conversation_events e JOIN conversation c ON c.id=e.conversation_id JOIN agent_runtime_bindings b ON b.id=$2 JOIN conversation_member human ON human.conv_id=c.id AND human.principal_id=$1 AND human.removed_at IS NULL JOIN conversation_member agent ON agent.conv_id=c.id AND agent.principal_id=b.agent_principal_id AND agent.removed_at IS NULL WHERE c.workspace_id=$3 AND e.event_id=ANY($4) AND e.event_type IN ('message','message.created','reply') AND (e.sender_id=$1 OR e.sender_id=b.agent_principal_id)", &[&claim.owner_id,&claim.binding_id,&claim.workspace_id,&ids]).await.map_err(storage)?;
        Ok(rows.iter().filter_map(|row| {
            let conversation: String = row.get("conversation_id");
            let sequence: i64 = row.get("seq");
            (sequence >= 0 && claim.cursor["feedback"][&conversation].as_u64().is_some_and(|cursor|sequence as u64 <= cursor))
                .then(||json!({"ref":format!("message:{}",row.get::<_,String>("event_id")),"record":{"content":row.get::<_,Option<String>>("content")}}))
        }).collect())
    }

    pub async fn configure_behavior_community(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        settings: &CommunitySettings,
    ) -> Result<(), AppError> {
        settings.validate().map_err(AppError::Validation)?;
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        if tx.execute("UPDATE experience_policy SET community_settings=$4,generation=generation+1,lease_token=NULL,lease_until=NULL,updated_at=NOW() WHERE workspace_id=$1 AND owner_id=$2 AND binding_id=$3", &[&workspace,&owner,&binding,&json!(settings)]).await.map_err(storage)? == 0 {
            return Err(AppError::NotFound("Learning policy not configured".into()));
        }
        tx.execute("UPDATE experience_behavior_event SET lease_token=NULL,lease_until=NULL WHERE workspace_id=$1 AND binding_id=$2 AND publication_state!='publishing'", &[&workspace,&binding]).await.map_err(storage)?;
        tx.commit().await.map_err(storage)
    }

    /// Retry only this owner's blocked, undispatched work with fresh review.
    /// Returns Conflict for other states; never duplicates an uncertain upload.
    pub async fn retry_behavior(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
        event: &str,
    ) -> Result<(), AppError> {
        let changed=self.store.connect().await?.execute("UPDATE experience_behavior_event e SET publication_state=CASE WHEN payload IS NULL THEN 'preparing' ELSE 'local' END,publication_error=NULL,public_payload=NULL,privacy_review=NULL,lease_token=NULL,lease_until=NULL,updated_at=NOW() FROM experience_policy p WHERE e.workspace_id=$1 AND e.binding_id=$3 AND e.id=$4 AND p.workspace_id=e.workspace_id AND p.binding_id=e.binding_id AND p.owner_id=$2 AND p.enabled AND e.publication_state='blocked' AND e.publication_url IS NULL", &[&workspace,&owner,&binding,&event]).await.map_err(storage)?;
        if changed == 0 {
            return Err(AppError::Conflict("Only your blocked, undispatched experience can be retried while learning is enabled".into()));
        }
        Ok(())
    }

    pub async fn behavior_community(
        &self,
        workspace: &str,
        owner: &str,
        binding: &str,
    ) -> Result<Value, AppError> {
        let client = self.store.connect().await?;
        let policy = client.query_opt("SELECT community_settings FROM experience_policy WHERE workspace_id=$1 AND owner_id=$2 AND binding_id=$3", &[&workspace,&owner,&binding]).await.map_err(storage)?;
        let settings: CommunitySettings = policy
            .map(|r| serde_json::from_value(r.get("community_settings")))
            .transpose()
            .map_err(|_| AppError::Internal("Invalid community settings".into()))?
            .unwrap_or_default();
        let rows = client.query("SELECT e.id,e.payload,e.publication_state,e.publication_url,e.publication_error FROM experience_behavior_event e JOIN experience_policy p ON p.binding_id=e.binding_id AND p.workspace_id=e.workspace_id WHERE e.workspace_id=$1 AND p.owner_id=$2 AND e.binding_id=$3 ORDER BY e.created_at DESC LIMIT 200", &[&workspace,&owner,&binding]).await.map_err(storage)?;
        let mut records = Vec::new();
        let mut entries = Vec::new();
        for row in rows {
            let payload: Option<Value> = row.get("payload");
            if let Some(payload) = &payload {
                records.push(
                    serde_json::from_value::<BehaviorRecord>(payload.clone())
                        .map_err(|_| AppError::Internal("Invalid stored behavior record".into()))?,
                );
            }
            entries.push(json!({"id":row.get::<_,String>("id"),"record":payload,"state":row.get::<_,String>("publication_state"),"url":row.get::<_,Option<String>>("publication_url"),"error":row.get::<_,Option<String>>("publication_error")}));
        }
        let public = if settings.search {
            client.query("SELECT payload FROM experience_community_record WHERE repository=$1 ORDER BY record_id LIMIT 200", &[&COMMUNITY_REPOSITORY]).await.map_err(storage)?.iter().map(|r|r.get::<_,Value>(0)).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let sync = client.query_opt("SELECT revision,checked_at::text,last_error FROM experience_community_sync WHERE repository=$1", &[&COMMUNITY_REPOSITORY]).await.map_err(storage)?;
        Ok(
            json!({"settings":settings,"local":entries,"counts":counts(&records),"count_scope":"displayed_records","community":public,"repository":COMMUNITY_REPOSITORY,
            "sync":sync.map(|r|json!({"revision":r.get::<_,Option<String>>(0),"checked_at":r.get::<_,Option<String>>(1),"error":r.get::<_,Option<String>>(2)}))}),
        )
    }

    pub async fn claim_behavior(&self) -> Result<Option<BehaviorClaim>, AppError> {
        let mut client = self.store.connect().await?;
        let tx = client.transaction().await.map_err(storage)?;
        let row = tx.query_opt("SELECT e.id,e.workspace_id,e.binding_id,e.problem_key,e.episode_ref,e.evidence_kind,e.solution_revision_id,e.source_references,p.owner_id,p.analyst_binding_id,p.source_cursor,p.generation,problem.description,problem.community_problem_id FROM experience_behavior_event e JOIN experience_policy p ON p.binding_id=e.binding_id AND p.workspace_id=e.workspace_id JOIN experience_problem problem ON problem.binding_id=e.binding_id AND problem.problem_key=e.problem_key WHERE p.enabled AND e.publication_state='preparing' AND (e.lease_until IS NULL OR e.lease_until<NOW()) ORDER BY e.created_at FOR UPDATE OF e SKIP LOCKED LIMIT 1", &[]).await.map_err(storage)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let id: String = row.get("id");
        let binding: String = row.get("binding_id");
        let key: String = row.get("problem_key");
        let workspace: String = row.get("workspace_id");
        let problem_id: String = tx.query_one("UPDATE experience_problem SET community_problem_id=COALESCE(community_problem_id,$4) WHERE binding_id=$1 AND problem_key=$2 AND workspace_id=$3 RETURNING community_problem_id", &[&binding,&key,&workspace,&choruz_common::new_id()]).await.map_err(storage)?.get(0);
        let token = choruz_common::new_id();
        tx.execute("UPDATE experience_behavior_event SET lease_token=$2,lease_until=NOW()+INTERVAL '10 minutes',updated_at=NOW() WHERE id=$1", &[&id,&token]).await.map_err(storage)?;
        let solution = tx.query_opt("SELECT r.id,r.instruction,r.validation FROM experience_problem p JOIN experience_revision r ON r.id=COALESCE($4,p.prompt_revision_id) AND r.workspace_id=p.workspace_id AND r.binding_id=p.binding_id WHERE p.workspace_id=$1 AND p.binding_id=$2 AND p.problem_key=$3 AND r.validation->>'review'='passed'", &[&workspace,&binding,&key,&row.get::<_,Option<String>>("solution_revision_id")]).await.map_err(storage)?.map(|r| {
            let validation: Value = r.get("validation");
            Ok::<_,AppError>(choruz_community::behavior::SolutionVersion {id:r.get("id"),instruction:r.get("instruction"),applicability:row.get("description"),
                based_on:validation["behavior_sources"].as_array().into_iter().flatten().filter_map(|source|source["candidate"]["record"]["solution"]["id"].as_str().map(str::to_owned)).collect(),
                team: if validation["team"]["review"]=="passed" { serde_json::from_value(validation["team"]["config"].clone()).map_err(|_|AppError::Internal("Invalid solution execution team".into()))? } else { None } })
        }).transpose()?;
        let occurrence_id: String = tx.query_one("SELECT id FROM experience_behavior_event WHERE workspace_id=$1 AND binding_id=$2 AND problem_key=$3 AND episode_ref=$4 ORDER BY created_at,id LIMIT 1", &[&workspace,&binding,&key,&row.get::<_,String>("episode_ref")]).await.map_err(storage)?.get(0);
        let claim = BehaviorClaim {
            id,
            token,
            workspace_id: workspace,
            owner_id: row.get("owner_id"),
            binding_id: binding,
            analyst_binding_id: row.get("analyst_binding_id"),
            problem_key: key,
            problem_id,
            description: row.get("description"),
            episode_ref: row.get("episode_ref"),
            references: row.get("source_references"),
            cursor: row.get("source_cursor"),
            applied_revision_id: row.get("solution_revision_id"),
            generation: row.get("generation"),
            solution,
            occurrence_id,
            kind: serde_json::from_value(json!(row.get::<_, String>("evidence_kind")))
                .map_err(|_| AppError::Internal("Invalid stored evidence kind".into()))?,
        };
        tx.commit().await.map_err(storage)?;
        Ok(Some(claim))
    }

    /// Persist an identity-preserving card only under the current policy lease.
    /// False fences stale preparation; malformed or relabeled evidence is rejected.
    pub async fn finish_behavior(
        &self,
        claim: &BehaviorClaim,
        record: Option<&BehaviorRecord>,
        error: Option<&str>,
    ) -> Result<bool, AppError> {
        if let Some(record) = record {
            record.validate().map_err(AppError::Validation)?;
            if record.id != claim.id
                || record.problem.id != claim.problem_id
                || record.occurrence_id != claim.occurrence_id
                || record.kind != claim.kind
                || record.solution != claim.solution
            {
                return Err(AppError::Validation(
                    "Prepared behavior changed its evidence identity".into(),
                ));
            }
        }
        let client = self.store.connect().await?;
        let changed = client.execute("UPDATE experience_behavior_event e SET payload=$4,publication_state=CASE WHEN $4::jsonb IS NULL THEN 'blocked' ELSE 'local' END,publication_error=$5,lease_token=NULL,lease_until=NULL,updated_at=NOW() FROM experience_policy p WHERE e.id=$1 AND e.workspace_id=$2 AND e.lease_token=$3 AND e.lease_until>NOW() AND p.binding_id=e.binding_id AND p.workspace_id=e.workspace_id AND p.enabled AND p.generation=$6", &[&claim.id,&claim.workspace_id,&claim.token,&record.map(|r|json!(r)),&error,&claim.generation]).await.map_err(storage)?;
        Ok(changed != 0)
    }
}

fn storage(error: tokio_postgres::Error) -> AppError {
    AppError::Internal(format!("Behavior library storage failed: {error}"))
}
