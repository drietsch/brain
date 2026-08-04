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
            // One shelf to begin with; the tree earns the rest once the
            // shape has been judged on a real mount.
            shelves: vec!["decisions"],
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
    fn records(&self, shelf: &str) -> FsResult<Vec<(String, String)>> {
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

    /// The bytes of one record: the document as the graph holds it.
    fn bytes(&self, shelf: &str, name: &str) -> FsResult<Vec<u8>> {
        let id = self
            .records(shelf)?
            .into_iter()
            .find(|(file, _)| file == name)
            .map(|(_, id)| id)
            .ok_or(FsError::NotFound)?;
        let view = self
            .state
            .read(|loaded| brain_eyes::query::thing::build(loaded, &id, None))
            .map_err(|_| FsError::GeneralFailure)?;
        view.body
            .and_then(|body| body.text)
            .map(String::into_bytes)
            .ok_or(FsError::NotFound)
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
