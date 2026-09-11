//! Empirical training-task performance for the current executor and baseline.
use super::DbService;
use choruz_common::AppError;
use choruz_domain::evaluation::TraceCase;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct TaskDifficulty {
    pub samples: usize,
    pub successes: usize,
}

impl TaskDifficulty {
    pub fn band(&self) -> &'static str {
        if self.samples < 3 {
            "insufficient"
        } else if self.successes * 5 >= self.samples * 4 {
            "easy"
        } else if self.successes * 5 <= self.samples {
            "hard"
        } else {
            "mixed"
        }
    }
}

impl DbService {
    pub async fn task_difficulty(
        &self,
        workspace: &str,
        binding: &str,
        context: &str,
        cases: &[TraceCase],
    ) -> Result<BTreeMap<String, TaskDifficulty>, AppError> {
        let client = self.store.connect().await?;
        // Bound the working history and project only score metadata: raw answers
        // and judge explanations are neither loaded nor given to the analyst.
        let rows=client.query("SELECT e.id, e.suite, e.optimization IS NOT NULL AS optimized, (SELECT COALESCE(jsonb_agg(jsonb_build_object('status',r->'status','candidate',r->'candidate','ordinal',r->'ordinal','case_id',r->'case_id','score',r->'score','error_code',r->'error_code')),'[]'::jsonb) FROM jsonb_array_elements(e.results) r) AS results FROM experience_evaluation e JOIN experience_policy p ON p.workspace_id=e.workspace_id AND p.binding_id=e.binding_id WHERE e.workspace_id=$1 AND e.binding_id=$2 AND e.context_fingerprint=$3 AND e.status='completed' AND e.candidates->0->>'revision_id' IS NOT DISTINCT FROM p.active_revision_id ORDER BY e.created_at DESC,e.id DESC LIMIT 100", &[&workspace,&binding,&context]).await.map_err(|e|AppError::Internal(format!("read task performance: {e}")))?;
        let mut result = BTreeMap::new();
        for row in rows {
            accumulate(
                &mut result,
                cases,
                &row.get::<_, Value>("suite"),
                &row.get::<_, Value>("results"),
                row.get("optimized"),
            );
        }
        Ok(result)
    }
}

fn accumulate(
    result: &mut BTreeMap<String, TaskDifficulty>,
    current: &[TraceCase],
    suite: &Value,
    results: &Value,
    optimized: bool,
) {
    let mut seen = BTreeSet::new();
    for r in results.as_array().into_iter().flatten() {
        let baseline = if optimized {
            r["candidate"] == 0
        } else {
            r["ordinal"].as_u64().is_some_and(|n| n % 2 == 0)
        };
        if !baseline || r["status"] != "completed" || !r["error_code"].is_null() {
            continue;
        }
        let Some(score) = r["score"].as_f64().filter(|s| *s == 0.0 || *s == 1.0) else {
            continue;
        };
        let Some(case) = suite["cases"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["id"] == r["case_id"] && c["split"] == "train")
        else {
            continue;
        };
        let Some(source) = current.iter().find(|c| json!(c) == case["source"]) else {
            continue;
        };
        let input = source.variant.as_ref().unwrap_or(&source.input);
        if case["input"] != *input
            || case["check"] != json!(source.check)
            || !seen.insert(source.episode_ref.clone())
        {
            continue;
        }
        let counts = result.entry(source.episode_ref.clone()).or_default();
        counts.samples += 1;
        counts.successes += usize::from(score == 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_current_completed_baseline_training_answers_count_once_per_run() {
        let source:TraceCase=serde_json::from_value(json!({"episode_ref":"task","evidence":["result"],"input":"Add 1 and 2","check":{"type":"exact","expected":"3"},"reason":"Verified"})).unwrap();
        let suite = json!({"cases":[{"id":"case","split":"train","input":source.input,"check":source.check,"source":source}]});
        let scored =
            json!({"status":"completed","candidate":0,"ordinal":0,"case_id":"case","score":0.0});
        let mut counts = BTreeMap::new();
        for _ in 0..3 {
            accumulate(
                &mut counts,
                std::slice::from_ref(&source),
                &suite,
                &json!([scored, scored]),
                true,
            );
        }
        assert_eq!(counts["task"].samples, 3);
        assert_eq!(counts["task"].band(), "hard");
        for (key, value) in [
            ("candidate", json!(1)),
            ("status", json!("inconclusive")),
            ("score", Value::Null),
            ("error_code", json!("timeout")),
        ] {
            let mut ignored = scored.clone();
            ignored[key] = value;
            accumulate(
                &mut counts,
                std::slice::from_ref(&source),
                &suite,
                &json!([ignored]),
                true,
            );
        }
        let mut holdout = suite.clone();
        holdout["cases"][0]["split"] = json!("validation");
        accumulate(
            &mut counts,
            std::slice::from_ref(&source),
            &holdout,
            &json!([scored]),
            true,
        );
        let mut changed = source.clone();
        changed.input = "Different task".into();
        accumulate(&mut counts, &[changed], &suite, &json!([scored]), true);
        assert_eq!(counts["task"].samples, 3);
        let mut manual = scored.clone();
        manual["ordinal"] = json!(1);
        accumulate(
            &mut counts,
            std::slice::from_ref(&source),
            &suite,
            &json!([manual]),
            false,
        );
        assert_eq!(counts["task"].samples, 3);
        accumulate(
            &mut counts,
            std::slice::from_ref(&source),
            &suite,
            &json!([scored]),
            false,
        );
        assert_eq!(counts["task"].samples, 4);
        assert_eq!(
            TaskDifficulty {
                samples: 2,
                successes: 0
            }
            .band(),
            "insufficient"
        );
        assert_eq!(
            TaskDifficulty {
                samples: 5,
                successes: 4
            }
            .band(),
            "easy"
        );
        assert_eq!(
            TaskDifficulty {
                samples: 5,
                successes: 2
            }
            .band(),
            "mixed"
        );
    }
}
