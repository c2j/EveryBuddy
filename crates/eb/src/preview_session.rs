//! The Read-preview document stack.
//!
//! This is the only stack. `App` keeps the preview mode and the native
//! `tool_preview`; it does not keep a copy of these documents. The HTTP
//! server in [`crate::preview_web_server`] serves this session.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const PREVIEW_CAP: usize = 5 * 1024 * 1024;
/// Open Read previews. A fifth file drops the oldest tab so the names stay readable.
pub(crate) const MAX_DOCS: usize = 4;

#[derive(Debug)]
pub enum ReadPreviewError {
    Io(std::io::Error),
    NotUtf8,
}

/// Identity of the file a preview was registered against.
/// Unix is `dev` + `ino`. Windows stores the volume serial in `dev` and
/// the file index in `ino`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileId {
    pub dev: u64,
    pub ino: u64,
}

/// One document the page can show. Built from the session for the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewDoc {
    pub id: [u8; 16],
    pub name: String,
    pub path: String,
    pub file_id: FileId,
}

/// Bytes are valid UTF-8. `truncated` means the file was longer than `cap`
/// and the cut was moved back to a character boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct PreviewText {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

impl PreviewText {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes).expect("read_preview only returns UTF-8")
    }
}

/// Read up to `cap` bytes. A longer file is cut on a UTF-8 boundary and
/// marked truncated. A file that is not UTF-8 is rejected, including when
/// the invalid sequence sits inside the kept prefix.
pub fn read_preview(path: &Path, cap: usize) -> Result<PreviewText, ReadPreviewError> {
    let file = File::open(path).map_err(ReadPreviewError::Io)?;
    read_opened(file, cap)
}

pub(crate) fn read_opened(file: File, cap: usize) -> Result<PreviewText, ReadPreviewError> {
    let mut buf = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(ReadPreviewError::Io)?;
    let truncated = buf.len() > cap;
    if truncated {
        buf.truncate(cap);
        // Only an incomplete sequence at the end is a boundary cut. An
        // illegal byte earlier in the prefix still fails the check below.
        if let Err(err) = std::str::from_utf8(&buf) {
            if err.error_len().is_none() {
                buf.truncate(err.valid_up_to());
            }
        }
    }
    if std::str::from_utf8(&buf).is_err() {
        return Err(ReadPreviewError::NotUtf8);
    }
    Ok(PreviewText {
        bytes: buf,
        truncated,
    })
}

pub fn file_identity(meta: &std::fs::Metadata) -> FileId {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        FileId {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        FileId {
            dev: u64::from(meta.volume_serial_number().unwrap_or(0)),
            ino: meta.file_index().unwrap_or(0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewEvent {
    Ready,
    Close,
    CloseTab { id: [u8; 16] },
    SelectTab { id: [u8; 16] },
    Error { message: String },
}

#[derive(Clone, Debug)]
pub(crate) enum Cache {
    Pending,
    Ready { bytes: Vec<u8>, truncated: bool },
    Failed { status: u16, message: &'static str },
}

pub(crate) struct Doc {
    pub(crate) path: PathBuf,
    pub(crate) file_id: FileId,
    pub(crate) name: String,
    pub(crate) generation: u64,
    pub(crate) cache: Cache,
}

pub struct PreviewSession {
    pub(crate) version: u64,
    pub(crate) order: Vec<[u8; 16]>,
    pub(crate) docs: HashMap<[u8; 16], Doc>,
    pub(crate) active: Option<[u8; 16]>,
    pub(crate) theme_dark: bool,
    ready: bool,
    last_error: Option<String>,
}

/// What the shell reads when it decides whether the panel is a web view.
/// Pending documents stay in `docs` so a cache miss is not a deleted file.
#[derive(Clone, Debug)]
pub(crate) struct SessionView {
    pub version: u64,
    pub docs: Vec<PreviewDoc>,
    pub active: Option<[u8; 16]>,
    pub ready: bool,
    pub last_error: Option<String>,
}

impl SessionView {
    /// The tab the page is showing, or the newest document.
    pub(crate) fn active_doc(&self) -> Option<&PreviewDoc> {
        self.active
            .and_then(|id| self.docs.iter().find(|doc| doc.id == id))
            .or_else(|| self.docs.last())
    }
}

impl PreviewSession {
    pub(crate) fn new() -> Self {
        Self {
            version: 0,
            order: Vec::new(),
            docs: HashMap::new(),
            active: None,
            theme_dark: false,
            ready: false,
            last_error: None,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn view(&self) -> SessionView {
        let docs = self
            .order
            .iter()
            .filter_map(|id| {
                let doc = self.docs.get(id)?;
                Some(PreviewDoc {
                    id: *id,
                    name: doc.name.clone(),
                    path: doc.path.display().to_string(),
                    file_id: doc.file_id,
                })
            })
            .collect();
        SessionView {
            version: self.version,
            docs,
            active: self.active,
            ready: self.ready,
            last_error: self.last_error.clone(),
        }
    }

    pub(crate) fn ready_text(&self, id: [u8; 16]) -> Option<(String, bool)> {
        let doc = self.docs.get(&id)?;
        match &doc.cache {
            Cache::Ready { bytes, truncated } => {
                let text = std::str::from_utf8(bytes).ok()?.to_string();
                Some((text, *truncated))
            }
            Cache::Pending | Cache::Failed { .. } => None,
        }
    }

    pub fn set_theme_dark(&mut self, dark: bool) {
        if self.theme_dark != dark {
            self.theme_dark = dark;
            self.version += 1;
        }
    }

    pub fn clear(&mut self) {
        if self.docs.is_empty() && self.order.is_empty() && self.active.is_none() {
            return;
        }
        self.docs.clear();
        self.order.clear();
        self.active = None;
        self.version += 1;
    }

    /// Register `path` without bumping `version`. A path already on the stack
    /// keeps its id; the previous bytes are dropped and the file is read again.
    /// A document past [`MAX_DOCS`] evicts the oldest and that eviction does bump.
    pub(crate) fn begin_doc(
        &mut self,
        path: PathBuf,
        file_id: FileId,
        name: String,
    ) -> ([u8; 16], u64) {
        if let Some(id) = self
            .order
            .iter()
            .copied()
            .find(|id| self.docs.get(id).is_some_and(|doc| doc.path == path))
        {
            let doc = self.docs.get_mut(&id).expect("id was just found");
            doc.file_id = file_id;
            doc.name = name;
            doc.generation += 1;
            doc.cache = Cache::Pending;
            self.active = Some(id);
            return (id, doc.generation);
        }
        if self.order.len() == MAX_DOCS {
            self.evict_oldest();
        }
        let id = random_16().expect("preview token source");
        let generation = 1;
        self.docs.insert(
            id,
            Doc {
                path,
                file_id,
                name,
                generation,
                cache: Cache::Pending,
            },
        );
        self.order.push(id);
        self.active = Some(id);
        (id, generation)
    }

    fn evict_oldest(&mut self) {
        let Some(id) = self.order.first().copied() else {
            return;
        };
        self.order.remove(0);
        self.docs.remove(&id);
        if self.active == Some(id) {
            self.active = self.order.first().copied();
        }
        self.version += 1;
    }
}

pub(crate) fn finish_read(
    session: &mut PreviewSession,
    id: [u8; 16],
    generation: u64,
    observed: Option<FileId>,
    result: Result<PreviewText, ReadPreviewError>,
) {
    let Some(doc) = session.docs.get_mut(&id) else {
        return;
    };
    if doc.generation != generation {
        return;
    }
    let registered = doc.file_id;
    doc.cache = match (observed, result) {
        (Some(fid), Ok(text)) if fid == registered => Cache::Ready {
            bytes: text.bytes,
            truncated: text.truncated,
        },
        (_, Err(ReadPreviewError::NotUtf8)) => Cache::Failed {
            status: 415,
            message: "not UTF-8",
        },
        (_, Ok(_)) => Cache::Failed {
            status: 409,
            message: "file changed",
        },
        (_, Err(ReadPreviewError::Io(_))) => Cache::Failed {
            status: 404,
            message: "cannot read file",
        },
    };
    session.version += 1;
}

pub(crate) fn apply_event(session: &mut PreviewSession, event: &PreviewEvent) {
    match event {
        PreviewEvent::Ready => session.ready = true,
        PreviewEvent::Close => session.clear(),
        PreviewEvent::CloseTab { id } => {
            let Some(pos) = session.order.iter().position(|item| item == id) else {
                return;
            };
            session.order.remove(pos);
            session.docs.remove(id);
            if session.active == Some(*id) {
                session.active = session
                    .order
                    .get(pos)
                    .copied()
                    .or_else(|| session.order.last().copied());
            }
            session.version += 1;
        }
        PreviewEvent::SelectTab { id } => {
            if !session.docs.contains_key(id) || session.active == Some(*id) {
                return;
            }
            session.active = Some(*id);
            session.version += 1;
        }
        PreviewEvent::Error { message } => session.last_error = Some(message.clone()),
    }
}

pub(crate) fn random_16() -> std::io::Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    #[cfg(unix)]
    {
        File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    }
    #[cfg(windows)]
    {
        extern "system" {
            fn SystemFunction036(buf: *mut u8, len: u32) -> u8;
        }
        let ok = unsafe { SystemFunction036(bytes.as_mut_ptr(), bytes.len() as u32) };
        if ok == 0 {
            return Err(std::io::Error::other("RtlGenRandom failed"));
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        compile_error!("preview token needs /dev/urandom or RtlGenRandom");
    }
    Ok(bytes)
}
