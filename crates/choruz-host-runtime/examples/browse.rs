use choruz_host_runtime::{FilesystemListing, HostRequest, execute};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir_in(std::env::current_dir()?)?;
    std::fs::create_dir(directory.path().join("project"))?;
    std::fs::write(directory.path().join("example.txt"), "owned example")?;
    let listing: FilesystemListing = serde_json::from_value(
        execute(HostRequest::FilesystemList {
            path: directory.path().to_string_lossy().into_owned(),
            show_hidden: false,
            include_files: true,
        })
        .await?,
    )?;
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.name == "project" && entry.kind == "directory")
    );
    assert!(
        listing
            .entries
            .iter()
            .any(|entry| entry.name == "example.txt" && entry.kind == "file")
    );
    println!("Listed an owned directory through HostRequest without a database or server.");
    Ok(())
}
