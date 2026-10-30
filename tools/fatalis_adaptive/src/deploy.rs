//! Conservative, single-file THK deployment prototype (not a hot-reload API).
//!
//! Only `game_root/nativePC/em/em013/00/data/em013_00.thk` is managed. Its parent
//! directories are created as needed after link checks. All tests use temporary directories. State
//! retains content-addressed generated THKs and original-file backups indefinitely.
//! Use a private state directory disjoint from the managed THK data directory.
//! `nativePC/plugins/FatalisAdaptive/data` is allowed for the DLL prototype.
//!
//! `install(source)` backs up the original; `update(transform)` transforms the
//! CURRENT owned bytes, not the backup; `restore()` checks ownership first.
//! Every mutating entry point first replays an interrupted transaction. Recovery
//! rolls forward only if the target equals its recorded before/after image.
//!
//! Atomic rename and durable intent provide crash recovery, not a distributed
//! transaction or protection against a hostile process writing the game file at
//! exactly the same instant. Stop the game/other mod managers before deployment.
//! Backup/restore preserves file bytes, not ACLs, timestamps, streams or metadata.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const RELATIVE_TARGET: &str = "nativePC/em/em013/00/data/em013_00.thk";
const FORMAT: u32 = 1;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub type Result<T> = std::result::Result<T, DeployError>;

#[derive(Debug)]
pub enum DeployError {
    Io(io::Error),
    Json(serde_json::Error),
    InvalidPath(String),
    Integrity(String),
    Conflict(String),
    AlreadyInstalled,
    NotInstalled,
    Busy,
}
impl fmt::Display for DeployError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "deployment I/O: {e}"),
            Self::Json(e) => write!(f, "deployment state JSON: {e}"),
            Self::InvalidPath(s) => write!(f, "unsafe deployment path: {s}"),
            Self::Integrity(s) => write!(f, "deployment state integrity: {s}"),
            Self::Conflict(s) => write!(f, "ownership conflict: {s}"),
            Self::AlreadyInstalled => write!(f, "already installed; use update"),
            Self::NotInstalled => write!(f, "not installed"),
            Self::Busy => write!(f, "another deployment operation is active"),
        }
    }
}
impl std::error::Error for DeployError {}
impl From<io::Error> for DeployError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for DeployError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installed {
    /// SHA-256 of the pre-install file, or None if no file existed.
    pub original_hash: Option<String>,
    /// SHA-256 of the most recently installed/updated THK.
    pub deployed_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    /// Binds private state to exactly one canonical target.
    target: PathBuf,
    installed: Option<Installed>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    before: State,
    after: State,
    before_hash: Option<String>,
    after_hash: Option<String>,
}

/// A handle does not keep a lock between calls. Each public operation takes an
/// OS-released-on-exit exclusive lock; abandoned locks do not block recovery.
#[derive(Debug)]
pub struct Deployer {
    game_root: PathBuf,
    state_dir: PathBuf,
    target: PathBuf,
}

impl Deployer {
    pub fn new(game_root: impl AsRef<Path>, state_dir: impl AsRef<Path>) -> Result<Self> {
        let game_root = fs::canonicalize(game_root)?;
        if !game_root.is_dir() {
            return Err(DeployError::InvalidPath(
                "game_root is not a directory".into(),
            ));
        }
        let target = game_root.join(RELATIVE_TARGET);
        ensure_target_parents(&game_root, &target)?;
        validate_target(&game_root, &target)?;
        let requested_state = resolve_future_path(state_dir.as_ref())?;
        validate_disjoint(
            &requested_state,
            target.parent().expect("fixed target parent"),
        )?;
        fs::create_dir_all(&requested_state)?;
        let state_dir = fs::canonicalize(&requested_state)?;
        validate_disjoint(&state_dir, target.parent().expect("fixed target parent"))?;
        let this = Self {
            game_root,
            state_dir,
            target,
        };
        let _lock = this.lock()?;
        this.validate_layout()?;
        // Bind even a never-installed state directory to its target. Otherwise a
        // state directory reused for another game could orphan its old ownership.
        if !this.metadata_path().exists() {
            if this.journal_path().exists() {
                return Err(DeployError::Integrity(
                    "journal without bound metadata".into(),
                ));
            }
            this.write_state(&this.empty_state())?;
        } else {
            this.read_state()?;
        }
        Ok(this)
    }

    pub fn target(&self) -> &Path {
        &self.target
    }
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Returns recorded ownership after recovery; does not claim an externally
    /// modified target is ours. Use `verify()` to check the actual bytes.
    pub fn installed(&self) -> Result<Option<Installed>> {
        let _lock = self.lock()?;
        self.recover_locked()?;
        Ok(self.read_state()?.installed)
    }

    pub fn verify(&self) -> Result<Option<Installed>> {
        let _lock = self.lock()?;
        self.recover_locked()?;
        let state = self.read_state()?;
        if let Some(ref installed) = state.installed {
            self.require_hash(Some(&installed.deployed_hash))?;
        }
        Ok(state.installed)
    }

    pub fn install(&self, source: impl AsRef<Path>) -> Result<Installed> {
        let _lock = self.lock()?;
        self.recover_locked()?;
        let before = self.read_state()?;
        if before.installed.is_some() {
            return Err(DeployError::AlreadyInstalled);
        }
        let generated = self.read_source(source.as_ref())?;
        let original = self.read_target()?;
        let original_hash = original.as_deref().map(hash);
        if let Some(ref bytes) = original {
            self.put_blob("backups", bytes)?;
        }
        let deployed_hash = self.put_blob("generated", &generated)?;
        let installed = Installed {
            original_hash: original_hash.clone(),
            deployed_hash: deployed_hash.clone(),
        };
        let mut after = before.clone();
        after.installed = Some(installed.clone());
        self.transact(Journal {
            version: FORMAT,
            before,
            after,
            before_hash: original_hash,
            after_hash: Some(deployed_hash),
        })?;
        Ok(installed)
    }

    /// The callback receives the verified CURRENT deployed THK. A rejected
    /// callback leaves ownership and the target unchanged. Do not recursively
    /// call deployment methods from this callback (the exclusive lock is held).
    pub fn update<F>(&self, transform: F) -> Result<Installed>
    where
        F: FnOnce(&[u8]) -> Result<Vec<u8>>,
    {
        let _lock = self.lock()?;
        self.recover_locked()?;
        self.update_locked(transform)
    }

    /// Explicit replacement alternative. Prefer `update` for patch generation,
    /// because its input is guaranteed to be the current managed THK.
    pub fn update_from(&self, source: impl AsRef<Path>) -> Result<Installed> {
        let _lock = self.lock()?;
        self.recover_locked()?;
        let generated = self.read_source(source.as_ref())?;
        self.update_locked(|_| Ok(generated))
    }

    fn update_locked<F>(&self, transform: F) -> Result<Installed>
    where
        F: FnOnce(&[u8]) -> Result<Vec<u8>>,
    {
        let before = self.read_state()?;
        let mut installed = before.installed.clone().ok_or(DeployError::NotInstalled)?;
        let current = self
            .read_target()?
            .ok_or_else(|| DeployError::Conflict("owned target is missing".into()))?;
        if hash(&current) != installed.deployed_hash {
            return Err(DeployError::Conflict(
                "target was changed outside this deployer".into(),
            ));
        }
        let generated = transform(&current)?;
        let previous_hash = installed.deployed_hash.clone();
        installed.deployed_hash = self.put_blob("generated", &generated)?;
        let mut after = before.clone();
        after.installed = Some(installed.clone());
        self.transact(Journal {
            version: FORMAT,
            before,
            after,
            before_hash: Some(previous_hash),
            after_hash: Some(installed.deployed_hash.clone()),
        })?;
        Ok(installed)
    }

    /// Restore the original bytes or remove only our owned file. No directory
    /// is ever removed. Repeated restore after a completed restore is a no-op.
    pub fn restore(&self) -> Result<()> {
        let _lock = self.lock()?;
        self.recover_locked()?;
        let before = self.read_state()?;
        let Some(installed) = before.installed.clone() else {
            return Ok(());
        };
        self.require_hash(Some(&installed.deployed_hash))?;
        let mut after = before.clone();
        after.installed = None;
        self.transact(Journal {
            version: FORMAT,
            before,
            after,
            before_hash: Some(installed.deployed_hash),
            after_hash: installed.original_hash,
        })
    }

    /// Roll a durable journal forward, refusing unrelated target bytes. Call
    /// explicitly on startup, or let another lifecycle operation call it.
    pub fn recover(&self) -> Result<()> {
        let _lock = self.lock()?;
        self.recover_locked()
    }

    fn empty_state(&self) -> State {
        State {
            version: FORMAT,
            target: self.target.clone(),
            installed: None,
        }
    }
    fn metadata_path(&self) -> PathBuf {
        self.state_dir.join("ownership.json")
    }
    fn journal_path(&self) -> PathBuf {
        self.state_dir.join("journal.json")
    }

    fn validate_layout(&self) -> Result<()> {
        validate_target(&self.game_root, &self.target)?;
        if !same_path(&fs::canonicalize(&self.state_dir)?, &self.state_dir) {
            return Err(DeployError::InvalidPath(
                "state directory was redirected".into(),
            ));
        }
        validate_disjoint(
            &self.state_dir,
            self.target.parent().expect("fixed target parent"),
        )?;
        for name in [
            "ownership.json",
            "journal.json",
            "lock",
            "generated",
            "backups",
        ] {
            reject_link(&self.state_dir.join(name))?;
        }
        Ok(())
    }
    fn lock(&self) -> Result<Lock> {
        self.validate_layout()?;
        Lock::acquire(&self.state_dir.join("lock"))
    }
    fn validate_state(&self, state: &State) -> Result<()> {
        if state.version != FORMAT || !same_path(&state.target, &self.target) {
            return Err(DeployError::Integrity(
                "state version/target mismatch".into(),
            ));
        }
        if let Some(ref i) = state.installed {
            validate_hash(&i.deployed_hash)?;
            if let Some(ref h) = i.original_hash {
                validate_hash(h)?;
            }
        }
        Ok(())
    }
    fn read_state(&self) -> Result<State> {
        reject_link(&self.metadata_path())?;
        let s: State = serde_json::from_slice(&fs::read(self.metadata_path())?)?;
        self.validate_state(&s)?;
        Ok(s)
    }
    fn write_state(&self, state: &State) -> Result<()> {
        self.validate_state(state)?;
        atomic_write_impl(&self.metadata_path(), &serde_json::to_vec_pretty(state)?)
    }
    fn read_target(&self) -> Result<Option<Vec<u8>>> {
        validate_target(&self.game_root, &self.target)?;
        read_optional(&self.target)
    }
    fn require_hash(&self, expected: Option<&String>) -> Result<()> {
        let actual = self.read_target()?.as_deref().map(hash);
        if actual.as_ref() != expected {
            return Err(DeployError::Conflict(
                "target does not match recorded ownership".into(),
            ));
        }
        Ok(())
    }
    fn read_source(&self, source: &Path) -> Result<Vec<u8>> {
        let source = fs::canonicalize(source)?;
        if same_path(&source, &self.target) || same_file(&source, &self.target)? {
            return Err(DeployError::InvalidPath(
                "source and target overlap (including hard links)".into(),
            ));
        }
        if !source.is_file() {
            return Err(DeployError::InvalidPath(
                "source is not a regular file".into(),
            ));
        }
        Ok(fs::read(source)?)
    }
    fn put_blob(&self, kind: &str, bytes: &[u8]) -> Result<String> {
        let digest = hash(bytes);
        let dir = self.state_dir.join(kind);
        reject_link(&dir)?;
        fs::create_dir_all(&dir)?;
        sync_directory(&self.state_dir)?;
        let path = dir.join(format!("{digest}.thk"));
        reject_link(&path)?;
        if path.exists() {
            if hash(&fs::read(&path)?) != digest {
                return Err(DeployError::Integrity(format!("corrupt {kind} blob")));
            }
        } else {
            atomic_write_impl(&path, bytes)?;
        }
        Ok(digest)
    }
    fn read_blob(&self, kind: &str, digest: &str) -> Result<Vec<u8>> {
        validate_hash(digest)?;
        let dir = self.state_dir.join(kind);
        reject_link(&dir)?;
        let path = dir.join(format!("{digest}.thk"));
        reject_link(&path)?;
        let bytes = fs::read(path)?;
        if hash(&bytes) != digest {
            return Err(DeployError::Integrity(format!("corrupt {kind} blob")));
        }
        Ok(bytes)
    }
    fn validate_journal(&self, j: &Journal) -> Result<()> {
        self.validate_state(&j.before)?;
        self.validate_state(&j.after)?;
        if j.version != FORMAT {
            return Err(DeployError::Integrity("unknown journal version".into()));
        }
        for h in [&j.before_hash, &j.after_hash].into_iter().flatten() {
            validate_hash(h)?;
        }
        let valid = match (&j.before.installed, &j.after.installed) {
            (None, Some(after)) => {
                after.original_hash == j.before_hash
                    && Some(&after.deployed_hash) == j.after_hash.as_ref()
            }
            (Some(before), Some(after)) => {
                before.original_hash == after.original_hash
                    && Some(&before.deployed_hash) == j.before_hash.as_ref()
                    && Some(&after.deployed_hash) == j.after_hash.as_ref()
            }
            (Some(before), None) => {
                Some(&before.deployed_hash) == j.before_hash.as_ref()
                    && before.original_hash == j.after_hash
            }
            (None, None) => false,
        };
        if !valid {
            return Err(DeployError::Integrity(
                "inconsistent journal transition".into(),
            ));
        }
        Ok(())
    }
    fn transact(&self, journal: Journal) -> Result<()> {
        self.validate_journal(&journal)?;
        self.require_hash(journal.before_hash.as_ref())?;
        // Durable intent MUST precede the target mutation.
        atomic_write_impl(&self.journal_path(), &serde_json::to_vec_pretty(&journal)?)?;
        self.recover_locked()
    }
    fn recover_locked(&self) -> Result<()> {
        self.validate_layout()?;
        let Some(bytes) = read_optional(&self.journal_path())? else {
            return Ok(());
        };
        let journal: Journal = serde_json::from_slice(&bytes)?;
        self.validate_journal(&journal)?;
        let state = self.read_state()?;
        if state != journal.before && state != journal.after {
            return Err(DeployError::Integrity(
                "journal disagrees with ownership metadata".into(),
            ));
        }
        let actual = self.read_target()?.as_deref().map(hash);
        if actual != journal.before_hash && actual != journal.after_hash {
            return Err(DeployError::Conflict(
                "interrupted transaction target was externally changed".into(),
            ));
        }
        if actual != journal.after_hash {
            let desired = if let Some(ref digest) = journal.after_hash {
                let kind = if journal.after.installed.is_some() {
                    "generated"
                } else {
                    "backups"
                };
                Some(self.read_blob(kind, digest)?)
            } else {
                None
            };
            // Recheck after potentially slow state reads, before replacing bytes.
            self.require_hash(journal.before_hash.as_ref())?;
            if let Some(ref bytes) = desired {
                atomic_write_impl(&self.target, bytes)?;
            } else {
                fs::remove_file(&self.target)?;
                sync_directory(self.target.parent().expect("fixed target has parent"))?;
            }
        }
        self.require_hash(journal.after_hash.as_ref())?;
        self.write_state(&journal.after)?;
        fs::remove_file(self.journal_path())?;
        sync_directory(&self.state_dir)?;
        Ok(())
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn validate_hash(s: &str) -> Result<()> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(DeployError::Integrity("invalid SHA-256 identifier".into()));
    }
    Ok(())
}
fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
fn path_within(path: &Path, parent: &Path) -> bool {
    let a: Vec<_> = path.components().collect();
    let b: Vec<_> = parent.components().collect();
    a.len() >= b.len()
        && a.iter()
            .zip(&b)
            .all(|(x, y)| same_path(Path::new(x.as_os_str()), Path::new(y.as_os_str())))
}
fn validate_disjoint(state: &Path, data: &Path) -> Result<()> {
    if path_within(state, data) || path_within(data, state) {
        return Err(DeployError::InvalidPath(
            "state must be disjoint from the managed THK data directory".into(),
        ));
    }
    Ok(())
}
fn reject_link(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                meta.file_attributes() & 0x400 != 0 // all reparse points, including junctions
            };
            #[cfg(not(windows))]
            let linked = meta.file_type().is_symlink();
            if linked {
                return Err(DeployError::InvalidPath(format!(
                    "link/reparse point: {}",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
/// Create only the fixed target's parents, checking every component before and after creation.
fn ensure_target_parents(root: &Path, target: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    for component in Path::new(RELATIVE_TARGET).components() {
        current.push(component.as_os_str());
        reject_link(&current)?;
        if same_path(&current, target) {
            break;
        }
        if !current.exists() {
            match fs::create_dir(&current) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        reject_link(&current)?;
        if !current.is_dir() {
            return Err(DeployError::InvalidPath(format!(
                "target parent is not a directory: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

fn validate_target(root: &Path, target: &Path) -> Result<()> {
    if !same_path(&fs::canonicalize(root)?, root) {
        return Err(DeployError::InvalidPath("game root was redirected".into()));
    }
    let mut current = root.to_path_buf();
    for component in Path::new(RELATIVE_TARGET).components() {
        current.push(component.as_os_str());
        reject_link(&current)?;
        if !same_path(&current, target) && !current.is_dir() {
            return Err(DeployError::InvalidPath(
                "target parent directories must already exist".into(),
            ));
        }
    }
    if target.exists() && !target.is_file() {
        return Err(DeployError::InvalidPath(
            "target is not a regular file".into(),
        ));
    }
    Ok(())
}
/// Canonicalize the existing prefix WITHOUT creating an unvalidated state path.
fn resolve_future_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Reject rather than normalize '..': existing symlink semantics are subtle.
    if absolute
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(DeployError::InvalidPath(
            "state path must not contain '..'".into(),
        ));
    }
    let mut existing = absolute.as_path();
    let mut tail: Vec<OsString> = Vec::new();
    while !existing.exists() {
        reject_link(existing)?;
        tail.push(
            existing
                .file_name()
                .ok_or_else(|| DeployError::InvalidPath("invalid state path".into()))?
                .to_os_string(),
        );
        existing = existing
            .parent()
            .ok_or_else(|| DeployError::InvalidPath("no existing state ancestor".into()))?;
    }
    let mut resolved = fs::canonicalize(existing)?;
    for part in tail.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}
fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    reject_link(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
/// General-purpose same-directory atomic persistence helper for runtime config.
/// Does not create parent directories; rejects a symlink/reparse destination.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::result::Result<(), String> {
    atomic_write_impl(path, bytes).map_err(|e| e.to_string())
}

fn atomic_write_impl(path: &Path, bytes: &[u8]) -> Result<()> {
    reject_link(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| DeployError::InvalidPath("write has no parent".into()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        ".fatalis-adaptive-{}-{stamp}-{nonce}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        reject_link(path)?;
        replace_atomic(&temp, path)?;
        sync_directory(parent)?;
        Ok(())
    })();
    // Never delete directories or sweep unknown files. A hard crash may leave a
    // uniquely named staging file; it is intentionally not guessed at on restart.
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
#[cfg(not(windows))]
fn replace_atomic(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}
#[cfg(windows)]
fn replace_atomic(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // Same-directory replacement, with no copy fallback, is atomic on supported
    // local Windows filesystems. WRITE_THROUGH requests durability.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
#[cfg(windows)]
fn sync_directory(_path: &Path) -> io::Result<()> {
    // Windows does not expose portable directory fsync; replacement uses
    // MOVEFILE_WRITE_THROUGH and every staged file is FlushFileBuffers'd.
    Ok(())
}

struct Lock {
    _file: File,
}
impl Lock {
    fn acquire(path: &Path) -> Result<Self> {
        reject_link(path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options.open(path).map_err(|e| {
            if e.kind() == io::ErrorKind::WouldBlock || e.raw_os_error() == Some(32) {
                DeployError::Busy
            } else {
                DeployError::Io(e)
            }
        })?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            extern "C" {
                fn flock(fd: i32, operation: i32) -> i32;
            }
            if unsafe { flock(file.as_raw_fd(), 2 | 4) } != 0 {
                let e = io::Error::last_os_error();
                return Err(if e.kind() == io::ErrorKind::WouldBlock {
                    DeployError::Busy
                } else {
                    e.into()
                });
            }
        }
        #[cfg(not(any(windows, unix)))]
        return Err(DeployError::InvalidPath(
            "platform lacks supported exclusive locking".into(),
        ));
        #[cfg(any(windows, unix))]
        Ok(Self { _file: file })
    }
}

fn same_file(a: &Path, b: &Path) -> Result<bool> {
    if !b.exists() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let a = fs::metadata(a)?;
        let b = fs::metadata(b)?;
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        #[derive(Default)]
        struct Info {
            attributes: u32,
            creation: [u32; 2],
            access: [u32; 2],
            write: [u32; 2],
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Info) -> i32;
        }
        let a = File::open(a)?;
        let b = File::open(b)?;
        let mut ai = Info::default();
        let mut bi = Info::default();
        if unsafe { GetFileInformationByHandle(a.as_raw_handle(), &mut ai) } == 0
            || unsafe { GetFileInformationByHandle(b.as_raw_handle(), &mut bi) } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok((ai.volume, ai.index_high, ai.index_low) == (bi.volume, bi.index_high, bi.index_low))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = a;
        Err(DeployError::InvalidPath(
            "platform lacks file identity checks".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        game: PathBuf,
        state: PathBuf,
        source: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "fatalis-deploy-test-{}-{stamp}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            let game = root.join("game");
            fs::create_dir_all(game.join(RELATIVE_TARGET).parent().unwrap()).unwrap();
            let source = root.join("input.thk");
            fs::write(&source, b"generated-v1").unwrap();
            Self {
                state: root.join("private"),
                root,
                game,
                source,
            }
        }
        fn target(&self) -> PathBuf {
            self.game.join(RELATIVE_TARGET)
        }
        fn deployer(&self) -> Deployer {
            Deployer::new(&self.game, &self.state).unwrap()
        }
        fn pending_install(&self, d: &Deployer) -> Journal {
            let before = d.read_state().unwrap();
            let original = fs::read(self.target()).ok();
            let before_hash = original.as_deref().map(hash);
            if let Some(bytes) = original {
                d.put_blob("backups", &bytes).unwrap();
            }
            let generated = d.put_blob("generated", b"generated-v1").unwrap();
            let mut after = before.clone();
            after.installed = Some(Installed {
                original_hash: before_hash.clone(),
                deployed_hash: generated.clone(),
            });
            let j = Journal {
                version: FORMAT,
                before,
                after,
                before_hash,
                after_hash: Some(generated),
            };
            atomic_write_impl(&d.journal_path(), &serde_json::to_vec(&j).unwrap()).unwrap();
            j
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Test-only cleanup is deliberately scoped to the unique temp root;
            // the production API never calls remove_dir/remove_dir_all.
            let canonical = fs::canonicalize(&self.root).unwrap();
            assert!(canonical.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
            fs::remove_dir_all(&canonical).unwrap();
        }
    }

    #[test]
    fn missing_entire_nativepc_tree_is_created_and_restore_keeps_directories() {
        let f = Fixture::new();
        let game = f.root.join("bare-game");
        fs::create_dir(&game).unwrap();
        assert!(!game.join("nativePC").exists());
        let d = Deployer::new(&game, f.root.join("bare-state")).unwrap();
        assert!(d.target().parent().unwrap().is_dir());
        d.install(&f.source).unwrap();
        assert_eq!(fs::read(d.target()).unwrap(), b"generated-v1");
        d.restore().unwrap();
        assert!(!d.target().exists());
        assert!(d.target().parent().unwrap().is_dir());
    }

    #[test]
    fn install_update_restore_original_and_keep_artifacts() {
        let f = Fixture::new();
        fs::write(f.target(), b"original").unwrap();
        let d = f.deployer();
        let first = d.install(&f.source).unwrap();
        assert_eq!(first.original_hash, Some(hash(b"original")));
        assert_eq!(fs::read(f.target()).unwrap(), b"generated-v1");
        let second = d
            .update(|current| {
                assert_eq!(current, b"generated-v1");
                let mut out = current.to_vec();
                out.extend_from_slice(b"+v2");
                Ok(out)
            })
            .unwrap();
        assert_eq!(second.deployed_hash, hash(b"generated-v1+v2"));
        assert_eq!(second.original_hash, first.original_hash);
        d.restore().unwrap();
        d.restore().unwrap();
        assert_eq!(fs::read(f.target()).unwrap(), b"original");
        assert!(d.read_blob("generated", &first.deployed_hash).is_ok());
        assert!(d.read_blob("generated", &second.deployed_hash).is_ok());
        assert_eq!(
            d.read_blob("backups", &hash(b"original")).unwrap(),
            b"original"
        );
        assert!(d.installed().unwrap().is_none());
    }

    #[test]
    fn absent_original_removes_only_file_and_preserves_directories() {
        let f = Fixture::new();
        let d = f.deployer();
        assert_eq!(d.install(&f.source).unwrap().original_hash, None);
        let parent = f.target().parent().unwrap().to_path_buf();
        fs::write(parent.join("unrelated.bin"), b"leave me").unwrap();
        d.restore().unwrap();
        assert!(!f.target().exists());
        assert!(parent.is_dir());
        assert_eq!(fs::read(parent.join("unrelated.bin")).unwrap(), b"leave me");
    }

    #[test]
    fn external_change_or_missing_owned_target_conflicts() {
        let f = Fixture::new();
        let d = f.deployer();
        d.install(&f.source).unwrap();
        fs::write(f.target(), b"other mod").unwrap();
        assert!(matches!(d.restore(), Err(DeployError::Conflict(_))));
        assert!(matches!(
            d.update(|_| Ok(vec![])),
            Err(DeployError::Conflict(_))
        ));
        assert!(matches!(d.verify(), Err(DeployError::Conflict(_))));
        assert_eq!(fs::read(f.target()).unwrap(), b"other mod");
        fs::remove_file(f.target()).unwrap();
        assert!(matches!(d.restore(), Err(DeployError::Conflict(_))));
    }

    #[test]
    fn reject_source_target_and_hardlink_alias() {
        let f = Fixture::new();
        fs::write(f.target(), b"original").unwrap();
        let d = f.deployer();
        assert!(matches!(
            d.install(f.target()),
            Err(DeployError::InvalidPath(_))
        ));
        let alias = f.root.join("alias.thk");
        fs::hard_link(f.target(), &alias).unwrap();
        assert!(matches!(d.install(alias), Err(DeployError::InvalidPath(_))));
        d.install(&f.source).unwrap();
        assert!(matches!(
            d.update_from(f.target()),
            Err(DeployError::InvalidPath(_))
        ));
    }

    #[test]
    fn reject_state_inside_managed_data_or_state_reuse() {
        let f = Fixture::new();
        let bad = f.game.join("nativePC/em/em013/00/data/private");
        assert!(matches!(
            Deployer::new(&f.game, &bad),
            Err(DeployError::InvalidPath(_))
        ));
        assert!(!bad.exists());
        f.deployer();
        let game2 = f.root.join("game2");
        fs::create_dir_all(game2.join(RELATIVE_TARGET).parent().unwrap()).unwrap();
        assert!(matches!(
            Deployer::new(game2, &f.state),
            Err(DeployError::Integrity(_))
        ));
    }

    #[test]
    fn dll_private_state_inside_plugins_is_allowed() {
        let f = Fixture::new();
        let state = f.game.join("nativePC/plugins/FatalisAdaptive/data");
        let d = Deployer::new(&f.game, state).unwrap();
        d.install(&f.source).unwrap();
        d.restore().unwrap();
    }

    #[test]
    fn lifecycle_errors_and_callback_failure_leave_target_unchanged() {
        let f = Fixture::new();
        let d = f.deployer();
        assert!(matches!(
            d.update(|_| Ok(vec![])),
            Err(DeployError::NotInstalled)
        ));
        d.install(&f.source).unwrap();
        assert!(matches!(
            d.install(&f.source),
            Err(DeployError::AlreadyInstalled)
        ));
        assert!(d
            .update(|_| Err(DeployError::Integrity("generation rejected".into())))
            .is_err());
        assert_eq!(fs::read(f.target()).unwrap(), b"generated-v1");
        assert!(!d.journal_path().exists());
    }

    #[test]
    fn recover_after_journal_before_target_write() {
        let f = Fixture::new();
        fs::write(f.target(), b"original").unwrap();
        let d = f.deployer();
        f.pending_install(&d);
        drop(d);
        let reopened = f.deployer();
        reopened.recover().unwrap();
        assert_eq!(fs::read(f.target()).unwrap(), b"generated-v1");
        reopened.restore().unwrap();
        assert_eq!(fs::read(f.target()).unwrap(), b"original");
    }

    #[test]
    fn recover_after_target_write_before_metadata_commit() {
        let f = Fixture::new();
        let d = f.deployer();
        let j = f.pending_install(&d);
        atomic_write_impl(&f.target(), b"generated-v1").unwrap();
        d.recover().unwrap();
        assert_eq!(d.installed().unwrap(), j.after.installed);
        assert!(!d.journal_path().exists());
        d.restore().unwrap();
        assert!(!f.target().exists());
    }

    #[test]
    fn recover_after_metadata_commit_before_journal_removal() {
        let f = Fixture::new();
        let d = f.deployer();
        let j = f.pending_install(&d);
        atomic_write_impl(&f.target(), b"generated-v1").unwrap();
        d.write_state(&j.after).unwrap();
        d.recover().unwrap();
        d.recover().unwrap();
        assert_eq!(d.installed().unwrap(), j.after.installed);
    }

    #[test]
    fn recover_interrupted_update_keeps_original_backup() {
        let f = Fixture::new();
        fs::write(f.target(), b"original").unwrap();
        let d = f.deployer();
        d.install(&f.source).unwrap();
        let before = d.read_state().unwrap();
        let next_hash = d.put_blob("generated", b"updated-v2").unwrap();
        let mut after = before.clone();
        after.installed.as_mut().unwrap().deployed_hash = next_hash.clone();
        let j = Journal {
            version: FORMAT,
            before_hash: Some(before.installed.as_ref().unwrap().deployed_hash.clone()),
            after_hash: Some(next_hash),
            before,
            after,
        };
        atomic_write_impl(&d.journal_path(), &serde_json::to_vec(&j).unwrap()).unwrap();
        d.recover().unwrap();
        assert_eq!(fs::read(f.target()).unwrap(), b"updated-v2");
        d.restore().unwrap();
        assert_eq!(fs::read(f.target()).unwrap(), b"original");
    }

    #[test]
    fn recover_interrupted_restore_with_and_without_original() {
        for original in [None, Some(b"original".as_slice())] {
            let f = Fixture::new();
            if let Some(bytes) = original {
                fs::write(f.target(), bytes).unwrap();
            }
            let d = f.deployer();
            d.install(&f.source).unwrap();
            let before = d.read_state().unwrap();
            let i = before.installed.clone().unwrap();
            let mut after = before.clone();
            after.installed = None;
            let j = Journal {
                version: FORMAT,
                before,
                after,
                before_hash: Some(i.deployed_hash),
                after_hash: i.original_hash,
            };
            atomic_write_impl(&d.journal_path(), &serde_json::to_vec(&j).unwrap()).unwrap();
            // Simulate crashing after the target transition, before metadata.
            if let Some(bytes) = original {
                atomic_write_impl(&f.target(), bytes).unwrap();
            } else {
                fs::remove_file(f.target()).unwrap();
            }
            d.recover().unwrap();
            assert_eq!(read_optional(&f.target()).unwrap().as_deref(), original);
            assert!(d.installed().unwrap().is_none());
        }
    }

    #[test]
    fn recovery_refuses_external_edits_and_keeps_journal() {
        let f = Fixture::new();
        let d = f.deployer();
        f.pending_install(&d);
        fs::write(f.target(), b"external").unwrap();
        assert!(matches!(d.recover(), Err(DeployError::Conflict(_))));
        assert_eq!(fs::read(f.target()).unwrap(), b"external");
        assert!(d.journal_path().exists());
    }

    #[test]
    fn corrupt_backup_is_rejected_without_touching_target() {
        let f = Fixture::new();
        fs::write(f.target(), b"original").unwrap();
        let d = f.deployer();
        d.install(&f.source).unwrap();
        fs::write(
            f.state
                .join("backups")
                .join(format!("{}.thk", hash(b"original"))),
            b"corrupt",
        )
        .unwrap();
        assert!(matches!(d.restore(), Err(DeployError::Integrity(_))));
        assert_eq!(fs::read(f.target()).unwrap(), b"generated-v1");
        assert!(d.journal_path().exists());
    }

    #[test]
    fn exclusive_lock_and_crash_style_release() {
        let f = Fixture::new();
        let d = f.deployer();
        let lock = d.lock().unwrap();
        assert!(matches!(d.recover(), Err(DeployError::Busy)));
        drop(lock);
        d.recover().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn reject_target_symlink_and_redirected_native_directory() {
        use std::os::unix::fs::symlink;
        let f = Fixture::new();
        symlink(&f.source, f.target()).unwrap();
        assert!(matches!(
            Deployer::new(&f.game, &f.state),
            Err(DeployError::InvalidPath(_))
        ));
        fs::remove_file(f.target()).unwrap();
        let data = f.target().parent().unwrap().to_path_buf();
        let moved = f.root.join("data-away");
        fs::rename(&data, &moved).unwrap();
        symlink(moved, data).unwrap();
        assert!(matches!(
            Deployer::new(&f.game, &f.state),
            Err(DeployError::InvalidPath(_))
        ));
    }
}
