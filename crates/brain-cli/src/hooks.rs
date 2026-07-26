//! `brain hook ...` — git integration: every commit/push triggers the brain.
//!
//! `install` writes three-line `post-commit` / `pre-push` hooks that call
//! back into `brain hook run <event>`, so the behavior lives in the binary
//! (improving it never needs a reinstall) and the hook files stay trivial.
//!
//! Hooks are **fail-open by design**: the twin is a sense organ, never a
//! gate. A brain failure must not block a commit or a push — `hook run`
//! swallows its own errors, and the hook script ends in `|| true`.

use brain_observe::twin;
use brain_store::Store;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER: &str = "# brain-hook";
const EVENTS: [&str; 2] = ["post-commit", "pre-push"];

pub fn cmd_hook(
    args: &[String],
    open_store: impl Fn() -> Result<Store, String>,
) -> Result<(), String> {
    let usage = "usage: brain hook install [dir] [--prefix <p>] [--docs] [--force] | \
                 uninstall [dir] | status [dir] | run <event> [--prefix <p>] [--docs]";
    match args.first().map(String::as_str) {
        Some("install") => {
            let (dir, prefix, docs, force) = parse(&args[1..])?;
            install(&dir, &prefix, docs, force)
        }
        Some("uninstall") => {
            let (dir, _, _, _) = parse(&args[1..])?;
            uninstall(&dir)
        }
        Some("status") => {
            let (dir, _, _, _) = parse(&args[1..])?;
            status(&dir)
        }
        Some("run") => {
            let event = args.get(1).ok_or(usage)?.clone();
            let (dir, prefix, docs, _) = parse(&args[2..])?;
            // Fail-open: report problems, never propagate them into git.
            if let Err(e) = run_event(&event, &dir, &prefix, docs, &open_store) {
                eprintln!("brain hook: {e} (ignored — hooks never block git)");
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

fn parse(args: &[String]) -> Result<(String, String, bool, bool), String> {
    let mut dir = ".".to_string();
    let mut prefix = "twin/self".to_string();
    let mut docs = false;
    let mut force = false;
    let mut positional = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--prefix" => prefix = it.next().cloned().ok_or("--prefix needs a value")?,
            "--docs" => docs = true,
            "--force" => force = true,
            other if !other.starts_with("--") && !positional => {
                dir = other.to_string();
                positional = true;
            }
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    Ok((dir, prefix, docs, force))
}

/// The repo's hooks directory, honoring worktrees (`git rev-parse`).
fn hooks_dir(dir: &str) -> Result<PathBuf, String> {
    let out = Command::new("git")
        .args(["-C", dir, "rev-parse", "--git-dir"])
        .output()
        .map_err(|e| format!("git not found: {e}"))?;
    if !out.status.success() {
        return Err(format!("'{dir}' is not a git repository"));
    }
    let git_dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let git_dir = if git_dir.is_absolute() { git_dir } else { Path::new(dir).join(git_dir) };
    Ok(git_dir.join("hooks"))
}

fn hook_body(event: &str, prefix: &str, docs: bool) -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "brain".to_string());
    let docs_flag = if docs && event == "pre-push" { " --docs" } else { "" };
    format!(
        "#!/bin/sh\n{MARKER} v1 — installed by `brain hook install`; the twin refreshes on\n\
         {MARKER} every {event}. Sense organ, never a gate: failures are ignored.\n\
         \"{exe}\" hook run {event} --prefix \"{prefix}\"{docs_flag} || true\n"
    )
}

fn install(dir: &str, prefix: &str, docs: bool, force: bool) -> Result<(), String> {
    let hooks = hooks_dir(dir)?;
    fs::create_dir_all(&hooks).map_err(|e| e.to_string())?;
    for event in EVENTS {
        let path = hooks.join(event);
        if path.exists() {
            let existing = fs::read_to_string(&path).unwrap_or_default();
            if !existing.contains(MARKER) && !force {
                return Err(format!(
                    "{} already has a non-brain hook; merge it manually or re-run with --force",
                    path.display()
                ));
            }
        }
        fs::write(&path, hook_body(event, prefix, docs)).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        println!("installed {}", path.display());
    }
    println!("every commit and push now refreshes the twin ({prefix})");
    Ok(())
}

fn uninstall(dir: &str) -> Result<(), String> {
    let hooks = hooks_dir(dir)?;
    for event in EVENTS {
        let path = hooks.join(event);
        if !path.exists() {
            continue;
        }
        let existing = fs::read_to_string(&path).unwrap_or_default();
        if existing.contains(MARKER) {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            println!("removed {}", path.display());
        } else {
            println!("kept {} (not a brain hook)", path.display());
        }
    }
    Ok(())
}

fn status(dir: &str) -> Result<(), String> {
    let hooks = hooks_dir(dir)?;
    for event in EVENTS {
        let path = hooks.join(event);
        let state = if !path.exists() {
            "absent"
        } else if fs::read_to_string(&path).unwrap_or_default().contains(MARKER) {
            "brain"
        } else {
            "foreign"
        };
        println!("{event:<12} {state}");
    }
    Ok(())
}

/// What actually happens on a git event: refresh the twin, then say — in
/// one or two lines — anything an author should know right now.
fn run_event(
    event: &str,
    dir: &str,
    prefix: &str,
    docs: bool,
    open_store: &impl Fn() -> Result<Store, String>,
) -> Result<(), String> {
    let store = open_store()?;
    let report =
        twin::refresh(&store, Path::new(dir), prefix).map_err(|e| e.to_string())?;
    println!(
        "brain[{event}]: {prefix} refreshed — {} added, {} changed, {} deleted, {} doc(s)",
        report.added.len(),
        report.changed.len(),
        report.deleted.len(),
        report.docs.len()
    );
    let ins = twin::insights(&store, prefix).map_err(|e| e.to_string())?;
    if !ins.stale_docs.is_empty() {
        println!(
            "brain[{event}]: {} possibly stale doc(s) — `brain twin stale {prefix}`",
            ins.stale_docs.len()
        );
    }
    if !ins.nonconforming.is_empty() {
        println!(
            "brain[{event}]: {} doc(s) violate their template — `brain twin insights {prefix}`",
            ins.nonconforming.len()
        );
    }
    if !ins.failing.is_empty() {
        println!(
            "brain[{event}]: {} test case(s) failing in the last imported protocol",
            ins.failing.len()
        );
    }
    if docs && event == "pre-push" {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let ok = Command::new(exe)
            .args(["docs", "generate", dir, "--prefix", prefix])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("brain[{event}]: docs regenerated (commit docs/generated changes next time)");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success());
        dir
    }

    #[test]
    fn install_status_uninstall_roundtrip() {
        let repo = git_repo();
        let dir = repo.path().to_str().unwrap();
        install(dir, "twin/app", false, false).unwrap();
        for event in EVENTS {
            let path = repo.path().join(".git/hooks").join(event);
            let body = fs::read_to_string(&path).unwrap();
            assert!(body.contains(MARKER));
            assert!(body.contains("hook run"));
            assert!(body.contains("twin/app"));
            assert!(body.ends_with("|| true\n"), "fail-open: {body}");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                assert!(fs::metadata(&path).unwrap().permissions().mode() & 0o111 != 0);
            }
        }
        // Re-install is an idempotent update, not an error.
        install(dir, "twin/app", true, false).unwrap();
        uninstall(dir).unwrap();
        for event in EVENTS {
            assert!(!repo.path().join(".git/hooks").join(event).exists());
        }
    }

    #[test]
    fn foreign_hooks_are_respected() {
        let repo = git_repo();
        let dir = repo.path().to_str().unwrap();
        let hooks = repo.path().join(".git/hooks");
        fs::create_dir_all(&hooks).unwrap();
        fs::write(hooks.join("pre-push"), "#!/bin/sh\necho custom\n").unwrap();

        let err = install(dir, "twin/app", false, false).unwrap_err();
        assert!(err.contains("non-brain hook"), "{err}");
        // --force overwrites; uninstall never removes what isn't ours.
        install(dir, "twin/app", false, true).unwrap();
        fs::write(hooks.join("post-commit"), "#!/bin/sh\necho other\n").unwrap();
        uninstall(dir).unwrap();
        assert!(hooks.join("post-commit").exists(), "foreign hook kept");
        assert!(!hooks.join("pre-push").exists(), "brain hook removed");
    }
}
