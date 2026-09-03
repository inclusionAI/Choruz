//! Agent keepalive scan.
//!
//! Every 60 seconds, scans companies with `agents_active = true`,
//! finds their agents' runtime bindings that have a session id,
//! and checks whether a PTY session exists in the pool.
//! Missing sessions are logged for now; actual rebuild happens
//! when the user opens the terminal.

use crate::PtyPool;

/// Spawn the keepalive background loop. Call once at startup.
pub(crate) fn spawn_keepalive_task(event_store: choruz_store::EventStore, pty_pool: PtyPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = keepalive_scan(&event_store, &pty_pool).await {
                tracing::warn!(error = %e, "keepalive scan failed");
            }
        }
    });
}

/// One pass of the keepalive scan.
async fn keepalive_scan(
    event_store: &choruz_store::EventStore,
    pty_pool: &PtyPool,
) -> Result<(), String> {
    let client = event_store
        .connect()
        .await
        .map_err(|e| format!("db connect: {e}"))?;

    // Find all bindings under companies with agents_active = true
    // that have a non-empty external_session_id and are not disabled.
    let rows = client
        .query(
            r#"
            SELECT b.id,
                   b.external_session_id,
                   b.workspace_path,
                   co.id   AS company_id,
                   co.name AS company_name
              FROM agent_runtime_bindings b
              JOIN company_member cm ON cm.principal_id = b.agent_principal_id
              JOIN company co        ON co.id = cm.company_id
             WHERE co.agents_active = TRUE
               AND b.state NOT IN ('disabled')
               AND b.external_session_id IS NOT NULL
               AND b.external_session_id != ''
            "#,
            &[],
        )
        .await
        .map_err(|e| format!("query bindings: {e}"))?;

    if rows.is_empty() {
        return Ok(());
    }

    // Snapshot the current PTY pool keys.
    let pool_keys: std::collections::HashSet<String> = {
        let pool = pty_pool.lock().expect("pool lock");
        pool.keys().cloned().collect()
    };

    let mut missing_count = 0u64;
    let total = rows.len();

    for row in &rows {
        let binding_id: String = row.get("id");
        let session_id: String = row.get("external_session_id");
        let company_name: String = row.get("company_name");

        // Check if this binding's session exists in the PTY pool.
        // The pool is keyed by binding_id.
        if !pool_keys.contains(&binding_id) {
            missing_count += 1;
            tracing::debug!(
                binding_id = %binding_id,
                session_id = %session_id,
                company = %company_name,
                "keepalive: PTY session missing for active binding"
            );
        }
    }

    if missing_count > 0 {
        tracing::info!(
            total_bindings = total,
            missing_pty = missing_count,
            "keepalive scan complete — {} binding(s) need PTY rebuild",
            missing_count,
        );
    } else {
        tracing::debug!(
            total_bindings = total,
            "keepalive scan complete — all PTY sessions present",
        );
    }

    Ok(())
}
