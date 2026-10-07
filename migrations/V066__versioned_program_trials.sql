ALTER TABLE experience_decision_trial ADD COLUMN context_key TEXT NOT NULL DEFAULT 'legacy';
ALTER TABLE experience_decision_trial ADD COLUMN context JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE experience_decision_trial ADD COLUMN state TEXT NOT NULL DEFAULT 'uncertain'
    CHECK(state IN ('reserved','generating','evaluating','completed','uncertain'));
ALTER TABLE experience_decision_trial ADD COLUMN outcome JSONB;
ALTER TABLE experience_decision_trial ADD COLUMN claim_token TEXT;
ALTER TABLE experience_decision_trial ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
ALTER TABLE experience_decision_trial DROP CONSTRAINT experience_decision_trial_pkey;
ALTER TABLE experience_decision_trial ADD PRIMARY KEY(workspace_id,binding_id,corpus,context_key);

WITH reports AS (
    SELECT trial.workspace_id,trial.binding_id,trial.corpus,
        (SELECT r.validation->'program_trial' FROM experience_revision r
         WHERE r.binding_id=trial.binding_id AND r.workspace_id=trial.workspace_id
           AND r.validation->'program_trial'->>'corpus'=trial.corpus
           AND r.validation->'program_trial'->>'status' IN ('validated','not_validated','not_suitable','browser_ready_for_current_task')
         ORDER BY r.created_at DESC LIMIT 1) AS result
    FROM experience_decision_trial trial
)
UPDATE experience_decision_trial trial SET state='completed',outcome=reports.result
FROM reports WHERE trial.workspace_id=reports.workspace_id AND trial.binding_id=reports.binding_id
    AND trial.corpus=reports.corpus AND reports.result IS NOT NULL;
