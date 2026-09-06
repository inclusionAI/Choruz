//! Directory browsing inside the device's browse roots, for the workspace
//! pickers. Roots are `CHORUZ_FS_BROWSE_ROOTS` (comma separated) or `HOME`,
//! compared in canonical form so a root that is itself a symlink admits the
//! paths below its target.

use std::path::{Path, PathBuf};

use choruz_common::AppError;
use serde::{Deserialize, Serialize};

pub const MAX_LISTED_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemHome {
    pub home: String,
    pub separator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<FilesystemEntry>,
}

pub fn browse_roots() -> Vec<PathBuf> {
    let configured = if let Ok(roots) = std::env::var("CHORUZ_FS_BROWSE_ROOTS") {
        roots
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect()
    } else {
        vec![
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/")),
        ]
    };
    configured
        .into_iter()
        .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
        .collect()
}

pub fn path_is_allowed(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

pub fn home_directory() -> FilesystemHome {
    FilesystemHome {
        home: std::env::var("HOME").unwrap_or_else(|_| "/".into()),
        separator: std::path::MAIN_SEPARATOR.to_string(),
    }
}

/// Canonicalise `path` and refuse it unless it lies inside a browse root.
pub fn allowed_canonical_path(path: &str) -> Result<PathBuf, AppError> {
    canonical_path_in_roots(path, &browse_roots())
}

fn canonical_path_in_roots(path: &str, roots: &[PathBuf]) -> Result<PathBuf, AppError> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| AppError::NotFound(format!("path not found: {error}")))?;
    if !path_is_allowed(&canonical, roots) {
        return Err(AppError::Forbidden(format!(
            "path {} is outside allowed browse roots",
            canonical.display()
        )));
    }
    Ok(canonical)
}

pub fn list_directory(
    path: &str,
    show_hidden: bool,
    include_files: bool,
) -> Result<FilesystemListing, AppError> {
    list_directory_in_roots(path, show_hidden, include_files, &browse_roots())
}

fn list_directory_in_roots(
    path: &str,
    show_hidden: bool,
    include_files: bool,
    roots: &[PathBuf],
) -> Result<FilesystemListing, AppError> {
    let canonical = canonical_path_in_roots(path, roots)?;
    let mut entries = Vec::new();
    let directory = std::fs::read_dir(&canonical)
        .map_err(|error| AppError::NotFound(format!("cannot read directory: {error}")))?;
    for entry in directory {
        let entry =
            entry.map_err(|error| AppError::Internal(format!("read directory entry: {error}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let mut kind = entry
            .file_type()
            .map_err(|error| AppError::Internal(format!("read file type: {error}")))?;
        if kind.is_symlink() {
            let Ok(target) = std::fs::canonicalize(entry.path()) else {
                continue;
            };
            if !path_is_allowed(&target, roots) {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(target) else {
                continue;
            };
            kind = metadata.file_type();
        }
        if !kind.is_dir() && !include_files {
            continue;
        }
        entries.push(FilesystemEntry {
            name,
            kind: if kind.is_dir() { "directory" } else { "file" }.into(),
            path: entry.path().to_string_lossy().into_owned(),
        });
        if entries.len() == MAX_LISTED_ENTRIES {
            break;
        }
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    let parent = canonical
        .parent()
        .filter(|parent| path_is_allowed(parent, roots))
        .map(|parent| parent.to_string_lossy().into_owned());
    Ok(FilesystemListing {
        path: canonical.to_string_lossy().into_owned(),
        parent,
        entries,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn directory_symlinks_follow_only_targets_within_browse_roots() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let target = root.join("target");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("plain.txt"), "plain").unwrap();
        std::fs::create_dir(root.join(".hidden")).unwrap();
        std::os::unix::fs::symlink(&target, root.join("alias")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("forbidden")).unwrap();
        std::os::unix::fs::symlink(root.join("missing"), root.join("broken")).unwrap();
        let roots = vec![std::fs::canonicalize(&root).unwrap()];
        for include_files in [false, true] {
            let listing =
                list_directory_in_roots(root.to_str().unwrap(), false, include_files, &roots)
                    .unwrap();
            assert_eq!(listing.parent, None);
            let alias = listing
                .entries
                .iter()
                .find(|entry| entry.name == "alias")
                .unwrap();
            assert_eq!(alias.kind, "directory");
            assert_eq!(alias.path, roots[0].join("alias").to_string_lossy());
            assert!(
                !listing
                    .entries
                    .iter()
                    .any(|entry| matches!(entry.name.as_str(), "forbidden" | "broken" | ".hidden"))
            );
            assert_eq!(
                listing
                    .entries
                    .iter()
                    .any(|entry| entry.name == "plain.txt"),
                include_files
            );
        }
        let listed =
            list_directory_in_roots(root.join("alias").to_str().unwrap(), false, false, &roots)
                .unwrap();
        assert_eq!(
            listed.path,
            std::fs::canonicalize(target).unwrap().to_string_lossy()
        );
        assert_eq!(listed.parent.as_deref(), roots[0].to_str());
        assert!(matches!(
            list_directory_in_roots(
                root.join("forbidden").to_str().unwrap(),
                false,
                false,
                &roots
            ),
            Err(AppError::Forbidden(_))
        ));
    }

    #[test]
    fn a_symlinked_browse_root_admits_the_paths_below_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        std::fs::create_dir_all(target.join("project")).unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let roots = vec![std::fs::canonicalize(&link).unwrap()];

        let listed = std::fs::canonicalize(link.join("project")).unwrap();
        assert!(path_is_allowed(&listed, &roots));
        assert!(!path_is_allowed(temp.path(), &roots));
        assert_eq!(roots[0], std::fs::canonicalize(&target).unwrap());
    }
}
