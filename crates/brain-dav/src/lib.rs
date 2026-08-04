//! The graph, mounted.
//!
//! A WebDAV surface over the same projection layer the cockpit renders,
//! so a decision read from a mounted file carries the sentences `say.rs`
//! composed for it — one voice, two surfaces.
//!
//! **There is no write verb.** `DavFileSystem`'s mutating methods
//! (`create_dir`, `remove_file`, `rename`, `copy`, `patch_props`) are left
//! at their default, which answers `NotImplemented`, and `open` refuses
//! anything but a read. That is the whole enforcement story: an agent
//! cannot create an artifact beside the real ones because the protocol it
//! is speaking has no way to ask. Authoring stays where it belongs —
//! `brain artifact new`, `brain testrun import` — and the mount shows the
//! result.
//!
//! Locking is answered by `FakeLs`, which exists for exactly one reason:
//! macOS and Windows refuse to mount a volume that will not grant a lock,
//! even when nothing will ever be written through it. The lock is
//! granted; the write behind it is not.

use dav_server::davpath::DavPath;
use dav_server::fs::{
    DavDirEntry, DavFile, DavFileSystem, DavMetaData, FsError, FsFuture, FsResult, FsStream,
    OpenOptions, ReadDirMeta,
};
use futures_util::{future, stream, FutureExt};
use std::io::SeekFrom;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use brain_eyes::AppState;

/// The shelves, mounted as directories of documents.
///
/// The tree is deliberately shallow to begin with: the root lists the
/// shelves, a shelf lists its records, a record is the document itself.
/// Everything is derived per request from the warm graph view, so a mount
/// is never stale and nothing has to be re-rendered to disk.
#[derive(Clone)]
pub struct GraphFs {
    state: Arc<AppState>,
    shelves: Vec<&'static str>,
}

impl GraphFs {
    pub fn new(state: Arc<AppState>) -> Box<GraphFs> {
        Box::new(GraphFs {
            state,
            shelves: vec!["decisions", "evidence", "tests"],
        })
    }

    /// What a path names: the root, a shelf, or a record on a shelf.
    fn resolve(&self, path: &DavPath) -> Node {
        let raw = path.as_pathbuf();
        let parts: Vec<String> = raw
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
                _ => None,
            })
            .collect();
        match parts.len() {
            0 => Node::Root,
            1 if self.shelves.contains(&parts[0].as_str()) => Node::Shelf(parts[0].clone()),
            2 if self.shelves.contains(&parts[0].as_str()) => {
                Node::Record(parts[0].clone(), parts[1].clone())
            }
            _ => Node::Missing,
        }
    }

    /// Every record on a shelf, as (file name, id).
    ///
    /// A shelf of writing lists the documents themselves. Evidence and
    /// tests have no file behind them — they are what the graph
    /// concluded — so each record is rendered on read, and the id is the
    /// key the renderer needs rather than an entity id.
    fn records(&self, shelf: &str) -> FsResult<Vec<(String, String)>> {
        match shelf {
            "evidence" => {
                let view = self
                    .state
                    .read(|loaded| loaded.evidence())
                    .map_err(|_| FsError::GeneralFailure)?;
                Ok(name_records(
                    view.claims
                        .iter()
                        .map(|claim| {
                            // A claim is a document *about* its subject,
                            // so it takes the subject's name and its own
                            // extension: a claim about app.js is app.md,
                            // never app.js.md.
                            let subject = claim
                                .subject
                                .as_ref()
                                .map(|r| r.label.clone())
                                .unwrap_or_else(|| claim.category.clone());
                            let base = subject.rsplit('/').next().unwrap_or(&subject);
                            let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
                            (format!("{stem}.md"), claim.id.clone())
                        })
                        .collect(),
                ))
            }
            "tests" => {
                let view = self
                    .state
                    .read(brain_eyes::query::tests::build)
                    .map_err(|_| FsError::GeneralFailure)?;
                let mut suites: Vec<String> = view
                    .cases
                    .iter()
                    .map(|case| case.group.clone())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                suites.sort();
                Ok(name_records(
                    suites
                        .into_iter()
                        .map(|group| (format!("{}.md", group.replace("::", "-")), group))
                        .collect(),
                ))
            }
            _ => {
                let view = self
                    .state
                    .read(|loaded| brain_eyes::query::library::build(loaded, shelf, ""))
                    .map_err(|_| FsError::GeneralFailure)?;
                let items = view
                    .items
                    .into_iter()
                    .map(|item| (item.label, item.id))
                    .collect::<Vec<_>>();
                Ok(name_records(items))
            }
        }
    }

    /// The bytes of one record: the document as the graph holds it, or as
    /// the graph would say it.
    fn bytes(&self, shelf: &str, name: &str) -> FsResult<Vec<u8>> {
        let key = self
            .records(shelf)?
            .into_iter()
            .find(|(file, _)| file == name)
            .map(|(_, key)| key)
            .ok_or(FsError::NotFound)?;
        let text = match shelf {
            "evidence" => self
                .state
                .read(|loaded| Ok(render_claim(&loaded.evidence()?, &key)))
                .map_err(|_| FsError::GeneralFailure)?
                .ok_or(FsError::NotFound)?,
            "tests" => self
                .state
                .read(|loaded| Ok(render_suite(&brain_eyes::query::tests::build(loaded)?, &key)))
                .map_err(|_| FsError::GeneralFailure)?
                .ok_or(FsError::NotFound)?,
            _ => self
                .state
                .read(|loaded| brain_eyes::query::thing::build(loaded, &key, None))
                .map_err(|_| FsError::GeneralFailure)?
                .body
                .and_then(|body| body.text)
                .ok_or(FsError::NotFound)?,
        };
        Ok(text.into_bytes())
    }
}

enum Node {
    Root,
    Shelf(String),
    Record(String, String),
    Missing,
}

/// What to say to something that tried to write here.
///
/// A refusal that only says "no" teaches nothing, and an agent that has
/// just been told "no" will try again. This one names the command that
/// does what the caller wanted — asked of the kind registry, the same
/// source the agent instructions are rendered from, so the mount cannot
/// recommend a route the policy does not have.
pub fn refusal(state: &AppState, method: &str, path: &str) -> String {
    let shelf = path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_string();
    // The derived shelves are not authored at all, and saying "use the
    // CLI" to something trying to write a test result would be useless
    // advice: a run is imported, never typed.
    let prefix = state
        .read(|loaded| Ok(loaded.prefix().to_string()))
        .unwrap_or_else(|_| "<prefix>".to_string());
    let derived: Vec<String> = match shelf.as_str() {
        "tests" => vec![
            format!("  a test result is recorded by importing a run: cargo test 2>&1 | brain testrun import - --prefix {prefix}"),
            format!("  a case no code declares any more is retired with: brain testrun purge {prefix}"),
            "  a result cannot be written by hand — it is evidence that something ran".to_string(),
        ],
        "evidence" => vec![
            "  evidence is not authored: a claim appears because some record makes it".to_string(),
            format!("  a claim is settled by making it true, or acknowledged with: brain artifact ack {prefix} <kind> <slug>"),
        ],
        _ => Vec::new(),
    };
    if !derived.is_empty() {
        let mut out = String::new();
        out.push_str("This mount is a read-only projection of the brain graph.\n");
        out.push_str(&format!("{method} {path} was refused; nothing was written.\n"));
        out.push_str("\nWhat you tried to do belongs to the CLI:\n");
        for line in derived {
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str("\nWhy: the graph is the system of record, so an artifact that appeared\n");
        out.push_str("beside the real ones without passing through it would be a claim with\n");
        out.push_str("no history, no source, and nothing corroborating it.\n");
        return out;
    }

    let route = state
        .read(|loaded| {
            let (_, _, kinds) = brain_eyes::query::library::shelf_kinds(&shelf);
            let registry = loaded.registry();
            let prefix = loaded.prefix().to_string();
            Ok(kinds
                .into_iter()
                .filter_map(|kind| registry.get(&kind).map(|def| (kind, def.clone())))
                .map(|(kind, def)| match brain_observe::kinds::author_via(&def, &prefix) {
                    Some(command) => format!("  a {kind} is authored with: {command}"),
                    None if def.placement == "projection" => {
                        format!("  a {kind} is a rendered query — it is never authored")
                    }
                    None => format!(
                        "  a {kind} is authored by writing its file ({}); the twin captures it",
                        if def.home.is_empty() {
                            "in the workspace".to_string()
                        } else {
                            def.home.join(", ")
                        }
                    ),
                })
                .collect::<Vec<_>>())
        })
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str("This mount is a read-only projection of the brain graph.\n");
    out.push_str(&format!("{method} {path} was refused; nothing was written.\n"));
    if route.is_empty() {
        out.push_str("\nRecords are authored through the brain CLI, never by writing here.\n");
    } else {
        out.push_str("\nWhat you tried to do belongs to the CLI:\n");
        for line in route {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("\nWhy: the graph is the system of record, so an artifact that appeared\n");
    out.push_str("beside the real ones without passing through it would be a claim with\n");
    out.push_str("no history, no source, and nothing corroborating it.\n");
    out
}

/// One claim, as a document.
///
/// Every sentence here was composed by the server for the cockpit; this
/// arranges them under headings and adds nothing. A claim that cannot
/// show its proof says so in its own words, and carries the command that
/// would settle it.
fn render_claim(view: &brain_eyes::dto::EvidenceView, id: &str) -> Option<String> {
    let claim = view.claims.iter().find(|c| c.id == id)?;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", claim.claim));
    out.push_str(&format!("{}\n\n", claim.verdict));
    if let Some(subject) = &claim.subject {
        out.push_str(&format!("About: {} ({})\n\n", subject.label, subject.noun));
    }
    out.push_str("## What stands behind it\n\n");
    if claim.proof.is_empty() {
        out.push_str("Nothing.\n");
    }
    for proof in &claim.proof {
        let mark = if proof.tone == "good" { "✓" } else { "✗" };
        out.push_str(&format!("- {mark} {}", proof.text));
        if let Some(basis) = &proof.basis {
            out.push_str(&format!(" — {basis}"));
        }
        out.push('\n');
    }
    if let Some(command) = &claim.fix_command {
        out.push_str(&format!("\n## What would settle it\n\n    {command}\n"));
    }
    out.push_str(&format!(
        "\n---\nRead from the graph. This file is a projection: {}\n",
        view.headline
    ));
    Some(out)
}

/// One suite, as a document: every case it holds, with its verdict.
///
/// A suite rather than a case, because a case is one line and a suite is
/// something you can read — and because 214 single-line files would be a
/// directory nobody opens twice.
fn render_suite(view: &brain_eyes::dto::TestsView, group: &str) -> Option<String> {
    let cases: Vec<_> = view.cases.iter().filter(|c| c.group == group).collect();
    if cases.is_empty() {
        return None;
    }
    let failing = cases.iter().filter(|c| c.result == "failing").count();
    let mut out = String::new();
    out.push_str(&format!("# {group}\n\n"));
    out.push_str(&format!(
        "{} case{}, {}.\n\n",
        cases.len(),
        if cases.len() == 1 { "" } else { "s" },
        if failing == 0 {
            "all passing".to_string()
        } else {
            format!("{failing} failing")
        }
    ));
    if let Some(kind) = cases.first().and_then(|c| c.kind_label.clone()) {
        let framework = cases
            .first()
            .and_then(|c| c.framework.clone())
            .unwrap_or_default();
        out.push_str(&format!("A {kind}, run by {framework}.\n\n"));
    }
    for case in &cases {
        let name = case.name.rsplit("::").next().unwrap_or(&case.name);
        out.push_str(&format!("## {name}\n\n"));
        out.push_str(&format!("{}", case.result));
        if let Some(duration) = &case.duration {
            out.push_str(&format!(" · took {duration}"));
        }
        if !case.when.is_empty() {
            out.push_str(&format!(" · since {}", case.when));
        }
        out.push_str("\n\n");
        if let Some(error) = &case.error {
            out.push_str(&format!("{error}\n\n"));
        }
        if let Some(note) = &case.note {
            out.push_str(&format!("{note}\n\n"));
        }
        for shot in &case.attachments {
            out.push_str(&format!("- left behind: {} ({})\n", shot.noun, shot.path));
        }
        if !case.attachments.is_empty() {
            out.push('\n');
        }
    }
    out.push_str(&format!("---\n{}\n", view.headline));
    Some(out)
}

/// A record's file name: its slug.
///
/// For anything that lives in a file, that is the file's own name, so
/// `adr-031-evidence-settles-applied-changes.md` on the mount is the same
/// string as on disk and in every command that names it. For records that
/// live only in the graph, the slug is the name and `.md` is added.
///
/// Two records can share a basename across directories. The slug is a
/// convenience for reading and typing; identity is the id, so a collision
/// is disambiguated rather than allowed to hide a record.
fn file_name(label: &str) -> String {
    let base = label.rsplit('/').next().unwrap_or(label);
    if base.contains('.') {
        base.to_string()
    } else {
        format!("{base}.md")
    }
}

/// Give every record a name of its own, in a stable order.
fn name_records(items: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    items
        .into_iter()
        .map(|(label, id)| {
            let wanted = file_name(&label);
            let count = seen.entry(wanted.clone()).or_insert(0);
            *count += 1;
            let name = if *count == 1 {
                wanted
            } else {
                match wanted.rsplit_once('.') {
                    Some((stem, ext)) => format!("{stem}-{count}.{ext}"),
                    None => format!("{wanted}-{count}"),
                }
            };
            (name, id)
        })
        .collect()
}

impl DavFileSystem for GraphFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        async move {
            // The mount is a projection. Anything that would write is
            // refused here rather than somewhere deeper.
            if options.write || options.append || options.truncate || options.create {
                return Err(FsError::Forbidden);
            }
            let Node::Record(shelf, name) = self.resolve(path) else {
                return Err(FsError::NotFound);
            };
            let bytes = self.bytes(&shelf, &name)?;
            Ok(Box::new(MemFile { bytes, at: 0 }) as Box<dyn DavFile>)
        }
        .boxed()
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        _meta: ReadDirMeta,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        async move {
            let entries: Vec<Box<dyn DavDirEntry>> = match self.resolve(path) {
                Node::Root => self
                    .shelves
                    .iter()
                    .map(|shelf| {
                        Box::new(Entry {
                            name: shelf.to_string(),
                            len: 0,
                            dir: true,
                        }) as Box<dyn DavDirEntry>
                    })
                    .collect(),
                Node::Shelf(shelf) => self
                    .records(&shelf)?
                    .into_iter()
                    .map(|(name, _)| {
                        Box::new(Entry {
                            name,
                            len: 0,
                            dir: false,
                        }) as Box<dyn DavDirEntry>
                    })
                    .collect(),
                _ => return Err(FsError::NotFound),
            };
            let stream = stream::iter(entries.into_iter().map(Ok));
            Ok(Box::pin(stream) as FsStream<Box<dyn DavDirEntry>>)
        }
        .boxed()
    }

    fn metadata<'a>(&'a self, path: &'a DavPath) -> FsFuture<'a, Box<dyn DavMetaData>> {
        async move {
            match self.resolve(path) {
                Node::Root | Node::Shelf(_) => {
                    Ok(Box::new(Meta { len: 0, dir: true }) as Box<dyn DavMetaData>)
                }
                Node::Record(shelf, name) => {
                    let len = self.bytes(&shelf, &name)?.len() as u64;
                    Ok(Box::new(Meta { len, dir: false }) as Box<dyn DavMetaData>)
                }
                Node::Missing => Err(FsError::NotFound),
            }
        }
        .boxed()
    }
}

#[derive(Debug, Clone)]
struct Meta {
    len: u64,
    dir: bool,
}

impl DavMetaData for Meta {
    fn len(&self) -> u64 {
        self.len
    }
    fn is_dir(&self) -> bool {
        self.dir
    }
    fn modified(&self) -> FsResult<SystemTime> {
        Ok(UNIX_EPOCH)
    }
}

struct Entry {
    name: String,
    len: u64,
    dir: bool,
}

impl DavDirEntry for Entry {
    fn name(&self) -> Vec<u8> {
        self.name.as_bytes().to_vec()
    }
    fn metadata(&self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        future::ready(Ok(Box::new(Meta {
            len: self.len,
            dir: self.dir,
        }) as Box<dyn DavMetaData>))
        .boxed()
    }
}

/// A record, rendered once and read from memory. Documents are small and
/// derived per request; holding one open costs nothing.
#[derive(Debug)]
struct MemFile {
    bytes: Vec<u8>,
    at: usize,
}

impl DavFile for MemFile {
    fn metadata(&mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        let len = self.bytes.len() as u64;
        future::ready(Ok(Box::new(Meta {
            len,
            dir: false,
        }) as Box<dyn DavMetaData>))
        .boxed()
    }
    fn read_bytes(&mut self, count: usize) -> FsFuture<'_, bytes::Bytes> {
        let end = (self.at + count).min(self.bytes.len());
        let chunk = bytes::Bytes::copy_from_slice(&self.bytes[self.at..end]);
        self.at = end;
        future::ready(Ok(chunk)).boxed()
    }
    fn seek(&mut self, pos: SeekFrom) -> FsFuture<'_, u64> {
        let len = self.bytes.len() as i64;
        let at = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => len + n,
            SeekFrom::Current(n) => self.at as i64 + n,
        };
        self.at = at.clamp(0, len) as usize;
        future::ready(Ok(self.at as u64)).boxed()
    }
    // Writing is not refused politely somewhere downstream; it is absent.
    fn write_buf(&mut self, _buf: Box<dyn bytes::Buf + Send>) -> FsFuture<'_, ()> {
        future::ready(Err(FsError::Forbidden)).boxed()
    }
    fn write_bytes(&mut self, _buf: bytes::Bytes) -> FsFuture<'_, ()> {
        future::ready(Err(FsError::Forbidden)).boxed()
    }
    fn flush(&mut self) -> FsFuture<'_, ()> {
        future::ready(Ok(())).boxed()
    }
}
