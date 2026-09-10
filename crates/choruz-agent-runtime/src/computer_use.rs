//! Device-installed computer-use skills remain available in isolated logins.
//! Only these skill directories are shared; credentials and Harness settings
//! remain owned by the selected account. Installation and OS consent stay
//! explicit host-operator actions, never side effects of an Agent launch.

use std::path::{Path, PathBuf};

/// Append the device's user-local executable directory without overriding
/// binaries already selected by the inherited PATH.
pub fn executable_path() -> Option<std::ffi::OsString> {
    let home = std::env::var_os("HOME")?;
    path_with_local_bin(&PathBuf::from(home), &std::env::var_os("PATH")?)
}

fn path_with_local_bin(home: &Path, inherited: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
    let local = home.join(".local/bin");
    if !local.is_dir() {
        return None;
    }
    let mut paths: Vec<_> = std::env::split_paths(inherited).collect();
    if !paths.contains(&local) {
        paths.push(local);
    }
    std::env::join_paths(paths).ok()
}

/// Make installed computer-use skills discoverable in an isolated profile.
/// Existing account skills take precedence. Missing installations are not
/// advertised and no download, browser connection or consent is attempted.
pub fn prepare_profile(profile: &Path) -> Result<(), String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Ok(());
    };
    link_skills(&PathBuf::from(home), profile)
        .map_err(|error| format!("prepare device computer-use skills: {error}"))
}

#[cfg(unix)]
fn link_skills(home: &Path, profile: &Path) -> std::io::Result<()> {
    use std::{fs, os::unix::fs::symlink};
    for name in ["browser-skill", "cua-driver"] {
        let disabled = home
            .join(".choruz/computer-use")
            .join(format!("{name}.disabled"))
            .exists();
        let target = profile.join("skills").join(name);
        let ownership = profile.join(".choruz-computer-use").join(name);
        if disabled {
            if let Ok(recorded) = fs::read_link(&ownership) {
                if fs::read_link(&target).ok().as_ref() == Some(&recorded) {
                    fs::remove_file(&target)?;
                }
                fs::remove_file(&ownership)?;
            }
            continue;
        }
        let source = [".agents/skills", ".claude/skills", ".codex/skills"]
            .into_iter()
            .map(|root| home.join(root).join(name))
            .find(|path| path.join("SKILL.md").is_file());
        let Some(source) = source else { continue };
        if fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        fs::create_dir_all(profile.join("skills"))?;
        match symlink(&source, target) {
            Ok(()) => {
                fs::create_dir_all(profile.join(".choruz-computer-use"))?;
                if fs::symlink_metadata(&ownership).is_ok() {
                    fs::remove_file(&ownership)?;
                }
                symlink(source, ownership)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn link_skills(_home: &Path, _profile: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn isolated_profile_shares_only_installed_skills_and_preserves_account_overrides() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("device-b");
        let profile = root.path().join("account-2/claude");
        let source = home.join(".agents/skills/browser-skill");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("SKILL.md"), "device B browser instructions").unwrap();
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("auth.json"), "account 2").unwrap();
        link_skills(&home, &profile).unwrap();
        assert_eq!(
            fs::read_to_string(profile.join("skills/browser-skill/SKILL.md")).unwrap(),
            "device B browser instructions"
        );
        assert_eq!(
            fs::read_to_string(profile.join("auth.json")).unwrap(),
            "account 2"
        );
        assert!(!profile.join("skills/cua-driver").exists());
        assert!(!profile.join("config.toml").exists());
        let custom = profile.join("skills/cua-driver");
        fs::create_dir_all(&custom).unwrap();
        fs::write(custom.join("SKILL.md"), "account override").unwrap();
        let installed = home.join(".agents/skills/cua-driver");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("SKILL.md"), "device skill").unwrap();
        link_skills(&home, &profile).unwrap();
        assert_eq!(
            fs::read_to_string(custom.join("SKILL.md")).unwrap(),
            "account override"
        );
        let switches = home.join(".choruz/computer-use");
        fs::create_dir_all(&switches).unwrap();
        for name in ["browser-skill", "cua-driver"] {
            fs::write(switches.join(format!("{name}.disabled")), "").unwrap();
        }
        link_skills(&home, &profile).unwrap();
        assert!(!profile.join("skills/browser-skill").exists());
        assert_eq!(
            fs::read_to_string(custom.join("SKILL.md")).unwrap(),
            "account override"
        );
        fs::remove_file(switches.join("browser-skill.disabled")).unwrap();
        link_skills(&home, &profile).unwrap();
        assert!(profile.join("skills/browser-skill/SKILL.md").is_file());
        let user_profile = root.path().join("custom-profile");
        fs::create_dir_all(user_profile.join("skills")).unwrap();
        std::os::unix::fs::symlink(&source, user_profile.join("skills/browser-skill")).unwrap();
        link_skills(&home, &user_profile).unwrap();
        fs::write(switches.join("browser-skill.disabled"), "").unwrap();
        fs::remove_file(source.join("SKILL.md")).unwrap();
        fs::remove_dir(&source).unwrap();
        link_skills(&home, &profile).unwrap();
        link_skills(&home, &user_profile).unwrap();
        assert!(fs::symlink_metadata(profile.join("skills/browser-skill")).is_err());
        assert!(
            fs::symlink_metadata(user_profile.join("skills/browser-skill"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn child_shell_finds_user_installed_tools_without_reordering_path() {
        let root = tempfile::tempdir().unwrap();
        let local = root.path().join(".local/bin");
        fs::create_dir_all(&local).unwrap();
        let tool = local.join("bsk");
        fs::write(&tool, "#!/bin/sh\nprintf device-local-browser").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let path = path_with_local_bin(root.path(), std::ffi::OsStr::new("/usr/bin:/bin")).unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", "bsk"])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"device-local-browser");
        let paths: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(
            paths,
            [PathBuf::from("/usr/bin"), PathBuf::from("/bin"), local]
        );
        assert_eq!(path_with_local_bin(root.path(), &path), Some(path));
    }
}
