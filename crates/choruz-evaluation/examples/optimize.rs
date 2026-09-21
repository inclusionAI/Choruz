use choruz_evaluation::{
    evaluation::{
        EvaluationCandidate, EvaluationCase, EvaluationSplit, EvaluationSuite, OutputCheck,
    },
    optimization::{Optimization, OptimizationConfig, SearchAction},
};

fn main() -> Result<(), String> {
    let suite = EvaluationSuite {
        name: "lowercase".into(),
        cases: [
            (EvaluationSplit::Train, "HELLO", "hello"),
            (EvaluationSplit::Validation, "WORLD", "world"),
            (EvaluationSplit::Test, "GOODBYE", "goodbye"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (split, input, expected))| EvaluationCase {
            id: i.to_string(),
            split,
            input: input.into(),
            check: OutputCheck::Exact {
                expected: expected.into(),
            },
            environment: None,
            source: None,
        })
        .collect(),
    };
    let mut search = Optimization::new(
        OptimizationConfig {
            max_metric_calls: 32,
            max_proposals: 2,
            minibatch_size: 1,
            seed: 7,
            merge: false,
            cache_evaluations: false,
            evolve_team: false,
            max_agents: 1,
        },
        &suite,
        ["echo", "lowercase"]
            .into_iter()
            .map(|instruction| EvaluationCandidate {
                revision_id: None,
                instruction: instruction.into(),
                team: None,
            })
            .collect(),
    )?;
    while let Some(action) = search.next_action(&suite)? {
        match action {
            SearchAction::Evaluate { candidate, case } => {
                let input = &suite.cases[case].input;
                let output = match search.candidates[candidate].guidance.instruction.as_str() {
                    "lowercase" => input.to_lowercase(),
                    _ => input.clone(),
                };
                let score = suite.cases[case]
                    .check
                    .score(&output)
                    .ok_or("This example requires deterministic checks")?;
                search.complete_scored_rollout(output, String::new(), score, None)?;
            }
            SearchAction::Propose { .. } => {
                // A real proposer receives this training-only payload, not the suite.
                let _feedback = search.proposal_input(&suite)?;
                search.complete_proposal("lowercase".into())?;
            }
        }
        let checkpoint = serde_json::to_string(&search).map_err(|e| e.to_string())?;
        search = serde_json::from_str(&checkpoint).map_err(|e| e.to_string())?;
    }
    assert!(search.can_apply(&suite));
    assert_eq!(
        search.application_scores(&suite),
        Some([0.0, 1.0, 0.0, 1.0])
    );
    println!("Selected lowercase; validation and held-out scores improved from 0 to 1.");
    Ok(())
}
