use choruz_common::AppError;
use choruz_evaluation::evaluation::OutputCheck;
use choruz_learning::{Runner, evaluate, judge, review};

struct Fixture;

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
    let task = "Add 7 and 8".to_owned();
    let answer = evaluate(
        &Fixture,
        task.clone(),
        "PRIVATE_GUIDANCE: check arithmetic".into(),
        String::new(),
    )
    .await?;
    let assessment = judge(
        &Fixture,
        task,
        OutputCheck::Judge {
            expected: "15".into(),
            rubric: "Return the correct sum".into(),
        },
        answer,
    )
    .await?;
    assert_eq!(assessment.score(), Some(1.0));
    println!(
        "Reviewed evidence and graded a separate task without leaking candidate guidance to its judge."
    );
    Ok(())
}
