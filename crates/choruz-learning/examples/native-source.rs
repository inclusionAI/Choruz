//! Read a synthetic account, without a platform host or authenticated CLI.
use choruz_learning::{
    native_source::{Harness, Source, read},
    source::Cursor,
};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("account");
    std::fs::create_dir_all(home.join("sessions"))?;
    std::fs::write(
        home.join("sessions/example.jsonl"),
        format!(
            "{}\n{}\n",
            json!({"type":"session_meta","payload":{"id":"example","cwd":root.path()}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":"Check the result"}})
        ),
    )?;
    let source = Source {
        harness: Harness::Codex,
        account_home: home,
        workspace_path: root.path().into(),
        session_id: "example".into(),
    };
    let window = read(&source, Cursor::default())?;
    assert_eq!(window.records.len(), 1);
    println!("{}", serde_json::to_string(&window)?);
    Ok(())
}
