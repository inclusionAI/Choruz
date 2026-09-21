use choruz_community::behavior::{BehaviorRecord, EvidenceKind, counts};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let encountered: BehaviorRecord = serde_json::from_value(json!({
        "schema_version": 1, "id": "event-1", "occurrence_id": "objective-1",
        "problem": {
            "id": "verification", "title": "Unsupported completion claim",
            "input_background": "A task required a successful check.",
            "expected_behavior": "Inspect the check before reporting completion.",
            "bad_behavior": "Reported completion without checking.",
            "applicability": "Tasks with explicit acceptance criteria", "tags": ["verification"]
        },
        "model": {"observed": null, "configured": "requested-model", "harness": "example", "harness_version": null},
        "solution": {"id": "solution-1", "based_on": [], "instruction": "Inspect required checks.", "team": null, "applicability": "Explicit acceptance checks"},
        "kind": "encountered", "evidence_summary": "A later correction identified the missing check."
    }))?;
    encountered.validate()?;
    let mut effective = encountered.clone();
    effective.id = "event-2".into();
    effective.kind = EvidenceKind::Effective;
    let mut disputed = effective.clone();
    disputed.id = "event-3".into();
    disputed.kind = EvidenceKind::Ineffective;
    let records = vec![encountered.clone(), encountered, effective, disputed];
    let persisted = serde_json::to_vec(&records)?;
    let restored: Vec<BehaviorRecord> = serde_json::from_slice(&persisted)?;
    for record in &restored {
        record.validate()?;
    }
    let result = counts(&restored);
    assert_eq!(result.encountered, 1);
    assert_eq!(result.applied, 1);
    assert_eq!(result.effective, 0);
    assert_eq!(result.ineffective, 1);
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
