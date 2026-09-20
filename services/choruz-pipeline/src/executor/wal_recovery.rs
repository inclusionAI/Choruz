//! Startup WAL reconciliation and its process-local operational evidence.

use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

use choruz_common::metrics::{self, Histogram, IntCounter, IntCounterVec, IntGauge};
use choruz_executor::wal::AdapterWal;

use super::ExecutorContext;

struct RecoveryMetrics {
    runs: IntCounterVec,
    errors: IntCounterVec,
    success: IntGauge,
    turns: IntCounter,
    duration: Histogram,
}

static METRICS: LazyLock<RecoveryMetrics> = LazyLock::new(|| {
    let runs = metrics::register_counter_vec(
        "choruz_wal_recovery_runs_total",
        "Completed WAL recovery scans by outcome.",
        &["outcome"],
    );
    for outcome in ["success", "failed"] {
        runs.with_label_values(&[outcome]);
    }
    let errors = metrics::register_counter_vec(
        "choruz_wal_recovery_errors_total",
        "WAL recovery errors by operation, not by file or user.",
        &["stage"],
    );
    for stage in ["directory", "enumerate", "open", "query", "record"] {
        errors.with_label_values(&[stage]);
    }
    RecoveryMetrics {
        runs,
        errors,
        success: metrics::register_gauge(
            "choruz_wal_recovery_success",
            "Whether the last WAL recovery scan completed without errors (1 or 0).",
        ),
        turns: metrics::register_counter(
            "choruz_wal_recovery_turns_total",
            "Incomplete WAL turns successfully marked failed during recovery.",
        ),
        duration: metrics::register_histogram(
            "choruz_wal_recovery_duration_seconds",
            "Duration of completed WAL recovery scans, including failed scans.",
            vec![0.001, 0.01, 0.1, 1.0, 10.0],
        ),
    }
});

#[derive(Default)]
struct RecoveryReport {
    databases: u64,
    incomplete: u64,
    recovered: u64,
    errors: u64,
}

impl RecoveryReport {
    fn error(&mut self, stage: &'static str, path: &Path, error: impl std::fmt::Display) {
        self.errors += 1;
        METRICS.errors.with_label_values(&[stage]).inc();
        tracing::warn!(stage, path = %path.display(), error = %error, "WAL recovery operation failed");
    }
}

impl ExecutorContext {
    /// Mark incomplete WAL turns failed, retaining errors while checking other files.
    ///
    /// This reconciles SQLite records, not session-manager retry state. A failed
    /// scan does not prevent pipeline startup; logs and metrics report degradation.
    pub async fn recover_from_wal(&self) {
        let started = Instant::now();
        LazyLock::force(&METRICS);
        let report = scan(&self.wal_base_dir).await;
        let success = report.errors == 0;
        let outcome = if success { "success" } else { "failed" };
        METRICS.runs.with_label_values(&[outcome]).inc();
        METRICS.success.set(i64::from(success));
        METRICS.turns.inc_by(report.recovered);
        METRICS.duration.observe(started.elapsed().as_secs_f64());
        if success {
            tracing::info!(
                outcome,
                databases = report.databases,
                incomplete = report.incomplete,
                recovered = report.recovered,
                errors = report.errors,
                "WAL crash recovery complete"
            );
        } else {
            tracing::error!(
                outcome,
                databases = report.databases,
                incomplete = report.incomplete,
                recovered = report.recovered,
                errors = report.errors,
                "WAL crash recovery incomplete; inspect WAL recovery operation failed records"
            );
        }
    }
}

async fn scan(base: &Path) -> RecoveryReport {
    let mut report = RecoveryReport::default();
    if let Err(error) = tokio::fs::create_dir_all(base).await {
        report.error("directory", base, error);
        return report;
    }
    let mut entries = match tokio::fs::read_dir(base).await {
        Ok(entries) => entries,
        Err(error) => {
            report.error("directory", base, error);
            return report;
        }
    };
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(error) => {
                report.error("enumerate", base, error);
                break;
            }
        };
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "db") {
            continue;
        }
        let wal = match AdapterWal::open(&path) {
            Ok(wal) => wal,
            Err(error) => {
                report.error("open", &path, error);
                continue;
            }
        };
        let incomplete = match wal.find_incomplete_turns().await {
            Ok(turns) => turns,
            Err(error) => {
                report.error("query", &path, error);
                continue;
            }
        };
        report.databases += 1;
        report.incomplete += incomplete.len() as u64;
        for turn in incomplete {
            tracing::warn!(turn_id = %turn.turn_id, attempt_id = %turn.attempt_id,
                path = %path.display(), "WAL recovery: marking incomplete turn failed");
            match wal
                .log_turn_failed(
                    &turn.turn_id,
                    &turn.attempt_id,
                    "executor crash recovery: process was not running",
                )
                .await
            {
                Ok(()) => report.recovered += 1,
                Err(error) => report.error("record", &path, error),
            }
        }
    }
    report
}
