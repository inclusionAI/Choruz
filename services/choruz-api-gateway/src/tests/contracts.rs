use std::{collections::BTreeSet, fs, path::Path};

/// Every path the gateway registers, read from the route table in `lib.rs`
/// and the plugin routers, with path parameters normalised to `{}`.
fn registered_paths() -> BTreeSet<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = vec![fs::read_to_string(src.join("lib.rs")).expect("read lib.rs")];
    for entry in fs::read_dir(src.join("plugins")).expect("list plugins") {
        let path = entry.expect("plugin entry").path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            sources.push(fs::read_to_string(&path).expect("read plugin"));
        }
    }
    let mut paths = BTreeSet::new();
    for source in sources {
        let mut rest = source.as_str();
        while let Some(start) = rest.find(".route(") {
            rest = &rest[start + ".route(".len()..];
            let literal = rest.trim_start();
            let Some(body) = literal.strip_prefix('"') else {
                continue;
            };
            let end = body.find('"').expect("route literal end");
            paths.insert(normalise(&body[..end]));
        }
    }
    paths
}

/// Every path `openapi/choruz.yaml` documents, normalised the same way.
fn documented_paths() -> BTreeSet<String> {
    let spec =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openapi/choruz.yaml"))
            .expect("read openapi/choruz.yaml");
    let mut in_paths = false;
    let mut paths = BTreeSet::new();
    for line in spec.lines() {
        if line == "paths:" {
            in_paths = true;
            continue;
        }
        if in_paths && !line.starts_with(' ') {
            break;
        }
        if in_paths
            && let Some(path) = line.strip_prefix("  /")
            && let Some(path) = path.strip_suffix(':')
        {
            paths.insert(normalise(&format!("/{path}")));
        }
    }
    paths
}

fn normalise(path: &str) -> String {
    let mut out = String::new();
    let mut in_param = false;
    for ch in path.chars() {
        match ch {
            '{' => {
                in_param = true;
                out.push_str("{}");
            }
            '}' => in_param = false,
            _ if in_param => {}
            _ => out.push(ch),
        }
    }
    out
}

/// `openapi/choruz.yaml` is the external contract: it lists every route the
/// gateway registers and nothing else.
#[test]
fn openapi_documents_every_route() {
    let registered = registered_paths();
    let documented = documented_paths();
    assert!(
        registered.len() > 50,
        "route table parse found {} paths",
        registered.len()
    );
    let undocumented: Vec<_> = registered.difference(&documented).collect();
    let phantom: Vec<_> = documented.difference(&registered).collect();
    assert!(
        undocumented.is_empty() && phantom.is_empty(),
        "openapi/choruz.yaml drifted from the route table.\nregistered but not documented: {undocumented:#?}\ndocumented but not registered: {phantom:#?}"
    );
}
