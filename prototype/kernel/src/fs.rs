//! The filesystem capability, served from the core.
//!
//! The local dsh backend resolves, stats, and reads through Node in the host
//! process. Here the same operations run inside the core through WASI preopens:
//! the host granted one directory at boot, and the core cannot name a path
//! outside it even if a plugin tries, because the WASI rights on the preopen are
//! the whole of what the core can exercise.
//!
//! A resolved target is an absolute canonical path. That matches the local
//! backend's realpath-shaped key and lets `processPath` answer synchronously.
use std::path::{Path, PathBuf};

use rutis::{BoxFuture, Ctx, Effect, Plugin};
use serde::Serialize;

use crate::host;

/// Longest file `read_text` will pull into wasm memory, over the JSON channel.
const MAX_READ_BYTES: u64 = 16 * 1024 * 1024;

/// Why a filesystem call failed. The host maps these onto `FsError` codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsFailureCode {
    /// The path does not exist.
    NotFound,
    /// The path exists but is not a regular file.
    NotRegularFile,
    /// The path resolved outside the preopened root.
    OutsideRoot,
    /// The file exceeds the read cap.
    TooLarge,
    /// The filesystem refused the operation.
    Io,
}

impl FsFailureCode {
    /// The `FsErrorCode` the host raises for this failure.
    pub fn fs_code(self) -> &'static str {
        match self {
            Self::NotFound => "FS_NOT_FOUND",
            Self::NotRegularFile => "FS_NOT_REGULAR_FILE",
            Self::OutsideRoot => "FS_PERMISSION_DENIED",
            Self::TooLarge => "FS_TOO_LARGE",
            Self::Io => "FS_IO_ERROR",
        }
    }
}

/// A filesystem failure with a host-mappable code.
#[derive(Debug)]
pub struct FsFailure {
    pub code: FsFailureCode,
    pub message: String,
}

impl FsFailure {
    fn new(code: FsFailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// One resolved target: the stable identity plus what a human sees.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTarget {
    /// Canonical absolute path; the same file yields the same key.
    pub target_key: String,
    /// Model/UI-facing path.
    pub display_path: String,
}

/// What `stat` reports about a present target.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatInfo {    /// Freshness token: size and mtime, so a concurrent write changes it.
    pub version: String,
    pub kind: String,
    /// Byte size of a regular file.
    pub size: Option<u64>,
}

/// The filesystem service, registered by [`FsProvider`].
pub struct FsService {
    root: PathBuf,
}

impl FsService {
    /// Confine all operations to `root`.
    ///
    /// The root is canonicalized so prefix checks compare canonical with
    /// canonical: on macOS a `/var/...` root would otherwise reject every
    /// resolved path, because `canonicalize` answers `/private/var/...`.
    pub fn new(root: PathBuf) -> Self {
        let root = root.canonicalize().unwrap_or(root);
        Self { root }
    }

    /// The root every path resolves under.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve `path` (absolute or relative to `cwd`) into a confined target.
    ///
    /// Absolute paths are accepted only when they already name something under
    /// the root, which is how a host hands back a path it got from this core.
    pub fn resolve(&self, path: &str, cwd: Option<&str>) -> Result<ResolvedTarget, FsFailure> {
        let raw = Path::new(path);
        let base = match (raw.is_absolute(), cwd) {
            (true, _) => raw.to_path_buf(),
            (false, Some(cwd)) => PathBuf::from(cwd).join(raw),
            (false, None) => self.root.join(raw),
        };
        let canonical = canonicalize(&base, path)?;
        if !canonical.starts_with(&self.root) {
            return Err(FsFailure::new(
                FsFailureCode::OutsideRoot,
                format!("\"{path}\" resolves outside the mounted root"),
            ));
        }
        let target_key = canonical.display().to_string();
        Ok(ResolvedTarget {
            display_path: target_key.clone(),
            target_key,
        })
    }

    /// Stat a resolved target; `None` when it no longer exists.
    pub fn stat(&self, target_key: &str) -> Result<Option<StatInfo>, FsFailure> {
        let path = self.confined(target_key)?;
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(FsFailure::new(FsFailureCode::Io, format!("stat {target_key}: {error}")))
            }
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_nanos())
            .unwrap_or_default();
        Ok(Some(StatInfo {
            version: format!("{}-{}", metadata.len(), modified),
            kind: if metadata.is_file() {
                "file".to_string()
            } else if metadata.is_dir() {
                "directory".to_string()
            } else {
                "other".to_string()
            },
            size: metadata.is_file().then_some(metadata.len()),
        }))
    }

    /// Read a regular file as UTF-8.
    pub fn read_text(&self, target_key: &str) -> Result<String, FsFailure> {
        let path = self.confined(target_key)?;
        let metadata = std::fs::metadata(&path)
            .map_err(|error| FsFailure::new(FsFailureCode::Io, format!("stat {target_key}: {error}")))?;
        if !metadata.is_file() {
            return Err(FsFailure::new(
                FsFailureCode::NotRegularFile,
                format!("{target_key} is not a regular file"),
            ));
        }
        if metadata.len() > MAX_READ_BYTES {
            return Err(FsFailure::new(
                FsFailureCode::TooLarge,
                format!("{target_key} is {} bytes, over the {} cap", metadata.len(), MAX_READ_BYTES),
            ));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| FsFailure::new(FsFailureCode::Io, format!("read {target_key}: {error}")))?;
        String::from_utf8(bytes)
            .map_err(|_| FsFailure::new(FsFailureCode::Io, format!("{target_key} is not UTF-8")))
    }

    /// The absolute path a host-side capability can open.
    pub fn process_path(&self, target_key: &str) -> Result<String, FsFailure> {
        self.confined(target_key).map(|path| path.display().to_string())
    }

    /// Whether `path` names an existing directory inside the mount.
    ///
    /// The search gate consults this per request; it canonicalizes like
    /// [`FsService::resolve`] does, so a symlinked alias of the mount answers yes.
    pub fn contains_dir(&self, path: &str) -> Result<bool, FsFailure> {
        let canonical = canonicalize(Path::new(path), path)?;
        Ok(canonical.starts_with(&self.root) && canonical.is_dir())
    }

    /// Refuse any key that is not a canonical path under the root.
    ///
    /// A key is only ever produced by [`FsService::resolve`], so an incoming key
    /// that does not prefix-match the root is either stale from another core or
    /// forged; both are the same failure.
    fn confined(&self, target_key: &str) -> Result<PathBuf, FsFailure> {
        let path = PathBuf::from(target_key);
        if path.starts_with(&self.root) {
            Ok(path)
        } else {
            Err(FsFailure::new(
                FsFailureCode::OutsideRoot,
                format!("{target_key} is outside the mounted root"),
            ))
        }
    }
}

/// Canonicalize `path`, naming escapes the way every caller reports them.
///
/// WASI refuses traversal above a preopen with ENOTCAPABLE (76): the escape was
/// stopped by the capability layer, so it is reported as [`FsFailureCode::OutsideRoot`]
/// instead of generic I/O. Native builds reach the prefix check in
/// [`FsService::resolve`] instead; both layers answer OutsideRoot.
fn canonicalize(path: &Path, display: &str) -> Result<PathBuf, FsFailure> {
    path.canonicalize()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => {
                FsFailure::new(FsFailureCode::NotFound, format!("\"{display}\" not found"))
            }
            _ if error.raw_os_error() == Some(76) => FsFailure::new(
                FsFailureCode::OutsideRoot,
                format!("\"{display}\" resolves outside the mounted root"),
            ),
            _ => FsFailure::new(FsFailureCode::Io, format!("resolve \"{display}\": {error}")),
        })
}

/// Registers [`FsService`] on the rutis registry.
///
/// The provider is a plugin, not a boot side effect, so the service obeys the
/// same lifecycle as every other registration: disposing it unregisters the
/// capability and every dependent sees the loss.
pub struct FsProvider {
    /// The preopened root handed to [`FsService`].
    pub root: PathBuf,
}

impl Plugin for FsProvider {
    fn name(&self) -> &str {
        "fs-provider"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, rutis::CordisError>> {
        Box::pin(async move {
            ctx.provide(FsService::new(self.root.clone()))?;
            host::log(
                "info",
                &format!("filesystem provider mounted {}", self.root.display()),
            );
            Ok(Effect::Done)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evobuddy-fs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_confines_to_the_root() {
        let root = scratch("confine");
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let fs = FsService::new(root.clone());
        // The backend canonicalizes the root, so every path it reports is
        // canonical (macOS mounts temp dirs under a symlinked prefix).
        let canonical_root = root.canonicalize().unwrap();

        let target = fs.resolve("a.txt", None).unwrap();
        assert_eq!(target.display_path, canonical_root.join("a.txt").display().to_string());

        let outside = root.parent().unwrap().join("outside.txt");
        std::fs::write(&outside, "x").unwrap();
        let failure = fs.resolve(outside.display().to_string().as_str(), None).unwrap_err();
        assert_eq!(failure.code, FsFailureCode::OutsideRoot);
        assert_eq!(failure.code.fs_code(), "FS_PERMISSION_DENIED");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stat_reports_kind_size_and_a_freshness_version() {
        let root = scratch("stat");
        std::fs::write(root.join("a.txt"), "hello").unwrap();
        let fs = FsService::new(root.clone());

        let key = fs.resolve("a.txt", None).unwrap().target_key;
        let info = fs.stat(&key).unwrap().unwrap();
        assert_eq!(info.kind, "file");
        assert_eq!(info.size, Some(5));
        assert!(!info.version.is_empty());

        // Keys come from `resolve`, so the absent probe uses a canonical path too.
        let absent = root.canonicalize().unwrap().join("absent.txt");
        assert!(fs.stat(&absent.display().to_string()).unwrap().is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn read_text_rejects_directories_and_oversize() {
        let root = scratch("read");
        std::fs::write(root.join("a.txt"), "line one\nline two\n").unwrap();
        std::fs::create_dir(root.join("d")).unwrap();
        let fs = FsService::new(root.clone());

        let key = fs.resolve("a.txt", None).unwrap().target_key;
        assert_eq!(fs.read_text(&key).unwrap(), "line one\nline two\n");

        let dir_key = fs.resolve("d", None).unwrap().target_key;
        assert_eq!(fs.read_text(&dir_key).unwrap_err().code, FsFailureCode::NotRegularFile);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
