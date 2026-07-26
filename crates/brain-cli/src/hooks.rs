//! `brain hook ...` — git integration: every commit/push triggers the brain.
//!
//! `install` writes three-line `post-commit` / `pre-push` hooks that call
//! back into `brain hook run <event>`, so the behavior lives in the binary
//! (improving it never needs a reinstall) and the hook files stay trivial.
//!
//! Hooks are **fail-open by design**: the twin is a sense organ, never a
//! gate. A brain failure must not block a commit or a push — `hook run`
//! swallows its own errors, and the hook script ends in `|| true`.

use brain_core::ids::StableId;
use brain_core::object::Object;
use brain_index::{replay, MemIndex};
use brain_observe::{testing, twin};
use brain_store::{now_ms, Store};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MARKER: &str = "# brain-hook";
const EVENTS: [&str; 2] = ["post-commit", "pre-push"];

#[derive(Debug, Default)]
struct Opts {
    dir: String,
    prefix: String,
    docs: bool,
    force: bool,
    tests: bool,
    test_cmd: Option<String>,
}

pub fn cmd_hook(
    args: &[String],
    open_store: impl Fn() -> Result<Store, String>,
) -> Result<(), String> {
    let usage = "usage: brain hook install [dir] [--prefix <p>] [--docs] [--tests] \
                 [--test-cmd <cmd>] [--force] | uninstall [dir] | status [dir] | \
                 run <event> [--prefix <p>] [--docs] [--tests]";
    match args.first().map(String::as_str) {
        Some("install") => {
            let opts = parse(&args[1..])?;
            if opts.tests {
                // The test command lives in the graph, not the hook file:
                // change it any time without reinstalling, and it
                // replicates with the twin.
                let cmd = opts
                    .test_cmd
                    .clone()
                    .or_else(|| infer_test_command(Path::new(&opts.dir)))
                    .ok_or("cannot infer a test command for this repo; pass --test-cmd \"...\"")?;
                set_test_command(&open_store()?, &opts.prefix, &cmd)?;
                println!("test command for {}: {cmd} (stored in the graph)", opts.prefix);
            }
            install(&opts)
        }
        Some("uninstall") => uninstall(&parse(&args[1..])?.dir),
        Some("status") => status(&parse(&args[1..])?.dir),
        Some("run") => {
            let event = args.get(1).ok_or(usage)?.clone();
            let opts = parse(&args[2..])?;
            // Fail-open: report problems, never propagate them into git.
            if let Err(e) = run_event(&event, &opts, &open_store) {
                eprintln!("brain hook: {e} (ignored — hooks never block git)");
            }
            Ok(())
        }
        _ => Err(usage.to_string()),
    }
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut opts = Opts {
        dir: ".".to_string(),
        prefix: "twin/self".to_string(),
        ..Opts::default()
    };
    let mut positional = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--prefix" => opts.prefix = it.next().cloned().ok_or("--prefix needs a value")?,
            "--docs" => opts.docs = true,
            "--force" => opts.force = true,
            "--tests" => opts.tests = true,
            "--test-cmd" => {
                opts.test_cmd = Some(it.next().cloned().ok_or("--test-cmd needs a value")?);
                opts.tests = true;
            }
            other if !other.starts_with("--") && !positional => {
                opts.dir = other.to_string();
                positional = true;
            }
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    Ok(opts)
}

/// Guess the obvious test command from the repo's manifests.
fn infer_test_command(dir: &Path) -> Option<String> {
    if dir.join("Cargo.toml").exists() {
        return Some("cargo test".to_string());
    }
    if dir.join("package.json").exists() {
        return Some("npm test".to_string());
    }
    if dir.join("pyproject.toml").exists() || dir.join("pytest.ini").exists() {
        return Some("pytest".to_string());
    }
    if dir.join("phpunit.xml").exists() || dir.join("phpunit.xml.dist").exists() {
        let vendored = dir.join("vendor/bin/phpunit");
        return Some(if vendored.exists() { "vendor/bin/phpunit".into() } else { "phpunit".into() });
    }
    None
}

/// Store (guarded) the repo's test command as a graph observation.
fn set_test_command(store: &Store, prefix: &str, cmd: &str) -> Result<(), String> {
    let mut index = MemIndex::new();
    replay(store, &mut index).map_err(|e| e.to_string())?;
    let repo_sid = StableId::derive(&["repo", prefix]);
    if twin::latest(&index, store, &repo_sid, "test_command")
        .map_err(|e| e.to_string())?
        .as_deref()
        != Some(cmd)
    {
        store
            .put(&Object::Observation {
                subject: repo_sid,
                property: "test_command".to_string(),
                value: cmd.to_string(),
                source: "hook".to_string(),
                observed_at_ms: now_ms(),
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
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

fn hook_body(event: &str, opts: &Opts) -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "brain".to_string());
    let docs_flag = if opts.docs && event == "pre-push" { " --docs" } else { "" };
    // Tests run post-commit (the moment code changed), not again on push.
    let tests_flag = if opts.tests && event == "post-commit" { " --tests" } else { "" };
    format!(
        "#!/bin/sh\n{MARKER} v1 — installed by `brain hook install`; the twin refreshes on\n\
         {MARKER} every {event}. Sense organ, never a gate: failures are ignored.\n\
         \"{exe}\" hook run {event} --prefix \"{prefix}\"{docs_flag}{tests_flag} || true\n",
        prefix = opts.prefix
    )
}

fn install(opts: &Opts) -> Result<(), String> {
    let hooks = hooks_dir(&opts.dir)?;
    fs::create_dir_all(&hooks).map_err(|e| e.to_string())?;
    for event in EVENTS {
        let path = hooks.join(event);
        if path.exists() {
            let existing = fs::read_to_string(&path).unwrap_or_default();
            if !existing.contains(MARKER) && !opts.force {
                return Err(format!(
                    "{} already has a non-brain hook; merge it manually or re-run with --force",
                    path.display()
                ));
            }
        }
        fs::write(&path, hook_body(event, opts)).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        println!("installed {}", path.display());
    }
    let extra = if opts.tests { " + tests" } else { "" };
    println!("every commit and push now refreshes the twin ({}{extra})", opts.prefix);
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

/// What actually happens on a git event: refresh the twin, run the
/// configured tests when asked, then say — in a few lines — anything an
/// author should know right now.
fn run_event(
    event: &str,
    opts: &Opts,
    open_store: &impl Fn() -> Result<Store, String>,
) -> Result<(), String> {
    let (dir, prefix, docs) = (opts.dir.as_str(), opts.prefix.as_str(), opts.docs);
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

    // Opt-in: run the graph-configured test command and import the
    // protocol, so every commit carries its test results automatically.
    if opts.tests && event == "post-commit" {
        match run_configured_tests(&store, dir, prefix)? {
            Some(out) => {
                let verdict = if out.failed == 0 { "ok" } else { "FAILED" };
                println!(
                    "brain[{event}]: protocol imported — {verdict}: {}/{} passed, {} failed, {} transition(s)",
                    out.passed, out.total, out.failed, out.transitions
                );
                for name in out.failing.iter().take(5) {
                    println!("brain[{event}]:   ✗ {name}");
                }
            }
            None => {}
        }
    }

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
    // The reflex points the eye: one line of top salience.
    {
        let mut index = MemIndex::new();
        replay(&store, &mut index).map_err(|e| e.to_string())?;
        if let Some(top) = brain_observe::attention::attend(&store, &index, prefix)
            .map_err(|e| e.to_string())?
            .first()
        {
            println!(
                "brain[{event}]: attention -> {} ({})",
                top.label,
                top.reasons.join(", ")
            );
        }
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

/// Run the test command stored on the repo entity and import its protocol.
/// Failing tests are recorded and reported, never a reason to block.
fn run_configured_tests(
    store: &Store,
    dir: &str,
    prefix: &str,
) -> Result<Option<testing::RunOutcome>, String> {
    let mut index = MemIndex::new();
    replay(store, &mut index).map_err(|e| e.to_string())?;
    let repo_sid = StableId::derive(&["repo", prefix]);
    let Some(cmd) = twin::latest(&index, store, &repo_sid, "test_command")
        .map_err(|e| e.to_string())?
    else {
        println!(
            "brain: no test command stored — re-run `brain hook install --tests --test-cmd \"...\"`"
        );
        return Ok(None);
    };
    println!("brain: running tests: {cmd}");
    let out = Command::new("sh")
        .args(["-c", &cmd])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run '{cmd}': {e}"))?;
    let mut raw = String::from_utf8_lossy(&out.stdout).into_owned();
    raw.push_str(&String::from_utf8_lossy(&out.stderr));
    let report = testing::parse_report(&raw);
    if report.cases.is_empty() {
        println!("brain: no test cases recognized in '{cmd}' output (cargo/JUnit expected)");
        return Ok(None);
    }
    let outcome = testing::record_run(store, prefix, &report, &raw).map_err(|e| e.to_string())?;
    Ok(Some(outcome))
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

    fn opts(dir: &str, tests: bool, force: bool) -> Opts {
        Opts {
            dir: dir.to_string(),
            prefix: "twin/app".to_string(),
            tests,
            force,
            ..Opts::default()
        }
    }

    #[test]
    fn install_status_uninstall_roundtrip() {
        let repo = git_repo();
        let dir = repo.path().to_str().unwrap();
        install(&opts(dir, false, false)).unwrap();
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
        install(&opts(dir, false, false)).unwrap();
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

        let err = install(&opts(dir, false, false)).unwrap_err();
        assert!(err.contains("non-brain hook"), "{err}");
        // --force overwrites; uninstall never removes what isn't ours.
        install(&opts(dir, false, true)).unwrap();
        fs::write(hooks.join("post-commit"), "#!/bin/sh\necho other\n").unwrap();
        uninstall(dir).unwrap();
        assert!(hooks.join("post-commit").exists(), "foreign hook kept");
        assert!(!hooks.join("pre-push").exists(), "brain hook removed");
    }

    #[test]
    fn tests_flag_lands_on_post_commit_only_and_command_is_inferred() {
        let repo = git_repo();
        let dir = repo.path().to_str().unwrap();
        fs::write(repo.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert_eq!(infer_test_command(repo.path()).as_deref(), Some("cargo test"));

        install(&opts(dir, true, false)).unwrap();
        let post = fs::read_to_string(repo.path().join(".git/hooks/post-commit")).unwrap();
        let push = fs::read_to_string(repo.path().join(".git/hooks/pre-push")).unwrap();
        assert!(post.contains("--tests"), "tests run at commit time: {post}");
        assert!(!push.contains("--tests"), "not re-run on push: {push}");

        // Other manifests infer their commands too.
        let py = tempfile::tempdir().unwrap();
        fs::write(py.path().join("pyproject.toml"), "").unwrap();
        assert_eq!(infer_test_command(py.path()).as_deref(), Some("pytest"));
        let none = tempfile::tempdir().unwrap();
        assert_eq!(infer_test_command(none.path()), None);
    }

    #[test]
    fn configured_tests_run_and_import_protocol() {
        let repo = git_repo();
        let dir = repo.path().to_str().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::open(store_dir.path()).unwrap();

        // No command stored yet: a hint, not an error.
        assert!(run_configured_tests(&store, dir, "twin/app").unwrap().is_none());

        set_test_command(
            &store,
            "twin/app",
            "printf 'test a::ok_case ... ok\\ntest b::bad_case ... FAILED\\n'",
        )
        .unwrap();
        let out = run_configured_tests(&store, dir, "twin/app").unwrap().unwrap();
        assert!(out.wrote);
        assert_eq!((out.total, out.passed, out.failed), (2, 1, 1));
        assert_eq!(out.failing, vec!["b::bad_case".to_string()]);

        // The protocol is in the graph: failing case queryable, and the
        // same output re-imports as a no-op (content-addressed run).
        let mut index = MemIndex::new();
        replay(&store, &mut index).unwrap();
        assert_eq!(
            testing::failing_cases(&store, &index, "twin/app").unwrap(),
            vec!["b::bad_case".to_string()]
        );
        let again = run_configured_tests(&store, dir, "twin/app").unwrap().unwrap();
        assert!(!again.wrote, "identical protocol is a no-op");

        // Changing the command is one guarded observation, no reinstall.
        set_test_command(&store, "twin/app", "printf 'test a::ok_case ... ok\\n'").unwrap();
        let out = run_configured_tests(&store, dir, "twin/app").unwrap().unwrap();
        assert_eq!(out.failed, 0);
    }
}
