//! Explicit, paid live check: `cargo run -p choruz-host-runtime --example experience_probe -- claude_terminal`.
use choruz_host_runtime::{TerminalSpec, experience::analyze};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let driver = std::env::args()
        .nth(1)
        .ok_or("provide claude_terminal or codex_terminal")?;
    let workspace = tempfile::tempdir()?;
    let spec = TerminalSpec {
        terminal_id: choruz_common::new_id(),
        driver_type: driver.clone(),
        binary_path: None,
        workspace_path: workspace.path().to_string_lossy().into(),
        cols: 120,
        rows: 40,
        resume_session_id: None,
        codex_home: None,
        model: std::env::args().nth(2).filter(|model| model != "default"),
        harness_account: json!({}),
    };
    let mode = std::env::args().nth(3);
    if matches!(mode.as_deref(), Some("team_serial" | "team_parallel")) {
        let result = choruz_host_runtime::harness::prepare(
            spec.clone(),
            &choruz_host_runtime::harness::ExecutionTeam {
                revision_id: "live-probe".into(),
                team: choruz_domain::team::Team {
                    order: if mode.as_deref() == Some("team_serial") { choruz_domain::team::Order::Serial } else { choruz_domain::team::Order::Parallel },
                    members: vec![
                        choruz_domain::team::Member { name:"derive".into(), prompt:"Compute the requested product. Give one short arithmetic derivation.".into() },
                        choruz_domain::team::Member { name:"check".into(), prompt:"Check the requested product independently and report any arithmetic discrepancy in one sentence.".into() },
                    ],
                },
            },
            "What is 17 times 19?",
        )
        .await?;
        assert!(result.contains("\"member\":\"derive\""));
        assert!(result.contains("\"member\":\"check\""));
        let output = choruz_host_runtime::experience::evaluate(
            spec,
            "What is 17 times 19?".into(),
            "Return only the decimal product, without commentary.".into(),
            result.clone(),
        )
        .await?;
        assert_eq!(output.trim(), "323");
        println!(
            "PASS {driver}: two collaborators and final executor completed {:?}\n{result}\nFinal output: {output}",
            mode
        );
        return Ok(());
    }
    if std::env::args().nth(3).as_deref() == Some("research") {
        let result = choruz_host_runtime::experience::research(
            spec,
            vec!["premature-completion-claim".into()],
        )
        .await?;
        println!("PASS {driver}: observed isolated web research\n{result}");
        return Ok(());
    }
    let prompt = format!(
        "{}\nInput:\n{}",
        include_str!("../../../agent-templates/experience-analysis.md"),
        json!({
            "current_instruction":"", "prior_summary":"", "active_revision_id":null,
            "trace":{"more":false,"records":[
                {"ref":"r1","role":"user","text":"Please compare the two approaches."},
                {"ref":"r2","role":"assistant","text":"Here is a long report."},
                {"ref":"r3","role":"user","text":"For future comparisons, start with a short recommendation, then explain the tradeoffs."}
            ]}
        })
    );
    let report = analyze(spec, prompt).await?;
    assert!(!report.summary.is_empty());
    assert!(
        report
            .evidence
            .iter()
            .all(|r| ["r1", "r2", "r3"].contains(&r.as_str()))
    );
    assert_eq!(report.previous_revision_outcome, "not_observed");
    println!(
        "PASS {driver}: isolated analysis returned a bounded, source-referenced report\n{}",
        serde_json::to_string_pretty(&report)?
    );
    Ok(())
}
