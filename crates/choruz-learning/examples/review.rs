use choruz_common::AppError;
use choruz_evaluation::evaluation::{
    EvaluationCandidate, EvaluationCase, EvaluationSplit, JudgeResult, OutputCheck,
};
use choruz_learning::{
    Runner, evaluate, judge, review,
    workflow::{EvaluationExecutor, ExecutionOutput, evaluate_task},
};

struct Fixture;

impl EvaluationExecutor for Fixture {
    async fn prepare(&self, _: &EvaluationCandidate, _: &str) -> Result<String, AppError> {
        unreachable!("This example evaluates one agent, not a team")
    }

    async fn execute(
        &self,
        candidate: &EvaluationCandidate,
        case: &EvaluationCase,
        preflight: &str,
    ) -> Result<ExecutionOutput, AppError> {
        Ok(ExecutionOutput {
            output: evaluate(
                self,
                case.input.clone(),
                candidate.instruction.clone(),
                preflight.into(),
            )
            .await?,
            replay: None,
            checks_passed: true,
        })
    }

    async fn judge(&self, case: &EvaluationCase, output: &str) -> Result<JudgeResult, AppError> {
        judge(self, case.input.clone(), case.check.clone(), output.into()).await
    }
}

impl Runner for Fixture {
    async fn run(
        &self,
        prompt: String,
        research: bool,
        system_prompt: &'static str,
    ) -> Result<String, AppError> {
        assert!(!research);
        if system_prompt == choruz_learning::REVIEW_SKILL {
            assert!(prompt.contains("source:1"));
            Ok(serde_json::json!({"accepted":true,"reason":"The observed correction supports checking arithmetic.","evidence":["source:1"],"addressed_problems":["unchecked-sum"]}).to_string())
        } else if system_prompt == choruz_learning::JUDGE_SKILL {
            assert!(prompt.contains("\"candidate_output\":\"15\""));
            assert!(!prompt.contains("PRIVATE_GUIDANCE"));
            Ok(
                serde_json::json!({"verdict":"pass","reason":"The sum matches the reference."})
                    .to_string(),
            )
        } else {
            assert!(prompt.contains("PRIVATE_GUIDANCE"));
            assert!(!prompt.contains("expected"));
            Ok("15".into())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let reviewed = review(
        &Fixture,
        "source:1: unchecked addition was corrected by the user".into(),
    )
    .await?;
    assert!(reviewed.accepted);
    let assessment = evaluate_task(
        &Fixture,
        &EvaluationCandidate {
            revision_id: None,
            instruction: "PRIVATE_GUIDANCE: check arithmetic".into(),
            team: None,
        },
        &EvaluationCase {
            id: "addition".into(),
            split: EvaluationSplit::Test,
            input: "Add 7 and 8".into(),
            source: None,
            environment: None,
            check: OutputCheck::Judge {
                expected: "15".into(),
                rubric: "Return the correct sum".into(),
            },
        },
    )
    .await?;
    assert_eq!(assessment["status"], "completed");
    assert_eq!(assessment["score"], 1.0);
    assert_eq!(assessment["output"], "15");
    println!(
        "Reviewed evidence and graded a separate task without leaking candidate guidance to its judge."
    );
    Ok(())
}
