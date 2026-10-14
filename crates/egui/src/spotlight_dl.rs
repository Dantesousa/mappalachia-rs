//! In-app spotlight tile downloader + installer (spotlight Task 5).
//!
//! The tiles ship as a split 7z pair on the upstream releases page (~3 GB).
//! This module downloads both volumes with progress, extracts them with
//! `sevenz-rust` through a chained [`ChainedReader`] spanning both files
//! (never `decompress_file` on `.001` alone — sevenz-rust has no
//! multi-volume support, see `.superpowers/sdd/spotlight-1-report.md`), and
//! verifies via the same rule as C# `FileIO.IsSpotlightInstalled`.
//!
//! Layout note: the archive payload sits under a top-level `spotlight/`
//! dir. Extraction goes into a staging dir
//! (`<assets>/img/.spotlight-staging/`) and the verified payload is
//! renamed into the live loader path `<assets>/img/spotlight/...` only
//! after verify — never straight into the live tree, so a cancelled or
//! failed run cannot leave partial tiles behind that would make the
//! installed-probe misreport `Installed: yes`.
//!
//! Threading mirrors the preview worker in `app.rs`: [`run`] does all
//! blocking work on a worker thread and only [`DlProgress`] crosses over an
//! `mpsc` channel. No `egui::Context` is touched off-thread. Cancelling is
//! dropping the receiver: every send is checked, and the worker aborts +
//! best-effort cleans the temp dir on the first failed send. No network
//! access happens in tests (pure state/reader/fs helpers only).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Spotlight patch volume 1 (upstream 2.0.5.2 release asset).
/// URLs change per game version — update both consts together.
pub const SPOTLIGHT_URL_001: &str =
    "https://github.com/AHeroicLlama/Mappalachia/releases/download/2.0.5.2/patch_spotlight.7z.001";
/// Spotlight patch volume 2 (upstream 2.0.5.2 release asset).
pub const SPOTLIGHT_URL_002: &str =
    "https://github.com/AHeroicLlama/Mappalachia/releases/download/2.0.5.2/patch_spotlight.7z.002";
/// Expected byte size of volume 1; mismatch after download is a failure.
pub const SPOTLIGHT_SIZE_001: u64 = 2146435072;
/// Expected byte size of volume 2; mismatch after download is a failure.
pub const SPOTLIGHT_SIZE_002: u64 = 819193318;
/// On-disk names of the two volumes inside the temp dir.
pub const SPOTLIGHT_FILE_001: &str = "patch_spotlight.7z.001";
/// On-disk names of the two volumes inside the temp dir.
pub const SPOTLIGHT_FILE_002: &str = "patch_spotlight.7z.002";
/// File entries in the archive (spike report: 466 `.jpg`); the extract
/// progress bar denominator. Directory entries are not counted.
pub const EXPECTED_FILE_COUNT: usize = 466;
/// Temp downloads live in `<assets>/img/.spotlight-dl/`, cleaned on
/// success, failure and cancel. No resume (YAGNI) — restart from scratch.
pub const DL_DIR_NAME: &str = ".spotlight-dl";
/// Extraction staging dir (`<assets>/img/.spotlight-staging/`): the
/// archive payload lands here as `<staging>/spotlight/...` and is renamed
/// into the live `<assets>/img/spotlight/` tree only after verify. Cancel
/// and failure paths remove the whole staging dir, so partial tiles never
/// reach the live tree (and a failed re-download never clobbers a good
/// existing install — the alternative fix, deleting the partial live tree
/// on failure, would do exactly that, which is why staging was chosen).
pub const STAGING_DIR_NAME: &str = ".spotlight-staging";
/// Progress sends at least this often during download (unbounded channel,
/// so the UI drains instead of queuing per-chunk updates).
const PROGRESS_EVERY_BYTES: u64 = 262_144;
/// Connect-phase bound for downloads.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-read bound for downloads (NOT a whole-download timeout): a ~3 GB
/// download takes minutes of steady progress, and every `read` that keeps
/// yielding bytes never trips this — but a stalled `read` errors out after
/// this long, so the worker reports `Failed` instead of lingering, and a
/// pending cancel (dropped receiver) is noticed on the very next send.
/// Set explicitly on the blocking client rather than relying on reqwest's
/// built-in default per-operation timeout, so a future builder change
/// cannot silently remove the bound.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Download/install lifecycle, owned by the UI thread. The worker reports
/// [`DlProgress`] and the UI folds each event in via [`DlState::apply`].
#[derive(Debug, Clone, PartialEq)]
pub enum DlState {
    Idle,
    Downloading {
        file_idx: usize,
        bytes_done: u64,
        bytes_total: u64,
    },
    Extracting {
        done: usize,
        total: usize,
    },
    Verifying,
    Done,
    Failed(String),
}

/// Worker-to-UI events. Terminal events are `Done` / `Failed`; anything
/// else keeps the progress UI repainting.
#[derive(Debug, Clone, PartialEq)]
pub enum DlProgress {
    Downloading {
        file_idx: usize,
        bytes_done: u64,
        bytes_total: u64,
    },
    Extracting {
        done: usize,
        total: usize,
    },
    Verifying,
    Done,
    Failed(String),
}

impl DlState {
    /// Fold one worker event into UI state (pure, unit-tested).
    pub fn apply(&mut self, progress: DlProgress) {
        match progress {
            DlProgress::Downloading {
                file_idx,
                bytes_done,
                bytes_total,
            } => {
                *self = DlState::Downloading {
                    file_idx,
                    bytes_done,
                    bytes_total,
                };
            }
            DlProgress::Extracting { done, total } => {
                *self = DlState::Extracting { done, total };
            }
            DlProgress::Verifying => *self = DlState::Verifying,
            DlProgress::Done => *self = DlState::Done,
            DlProgress::Failed(message) => *self = DlState::Failed(message),
        }
    }

    /// True while the worker may still send (download/extract/verify).
    /// The panel shows the Cancel button exactly then.
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            DlState::Downloading { .. } | DlState::Extracting { .. } | DlState::Verifying
        )
    }
}

/// How a blocking step aborted: user-cancelled (receiver gone, stay quiet)
/// or a real failure (report it when anyone is still listening).
#[derive(Debug)]
enum Abort {
    Cancelled,
    Failed(String),
}

/// Send one progress event; a dead receiver means the user cancelled.
fn send(tx: &Sender<DlProgress>, progress: DlProgress) -> Result<(), Abort> {
    tx.send(progress).map_err(|_| Abort::Cancelled)
}

/// Temp dir for the two downloaded volumes.
pub fn dl_dir(assets_root: &Path) -> PathBuf {
    assets_root.join("img").join(DL_DIR_NAME)
}

/// Best-effort temp cleanup (success/failure/cancel paths share it).
/// Never fails: a missing dir is already clean.
pub fn clean_dl_dir(assets_root: &Path) {
    let _ = std::fs::remove_dir_all(dl_dir(assets_root));
}

/// Staging dir for extraction (see [`STAGING_DIR_NAME`]).
pub fn staging_dir(assets_root: &Path) -> PathBuf {
    assets_root.join("img").join(STAGING_DIR_NAME)
}

/// Best-effort staging cleanup (success/failure/cancel paths share it,
/// alongside [`clean_dl_dir`]). Never fails.
pub fn clean_staging_dir(assets_root: &Path) {
    let _ = std::fs::remove_dir_all(staging_dir(assets_root));
}

/// Shared HTTP client: connect + per-read timeouts, no whole-download
/// timeout (a ~3 GB download takes minutes). Split out so tests can assert
/// it builds without touching the network.
fn build_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(READ_TIMEOUT)
        .build()
        .map_err(|e| format!("cannot start download: {e}"))
}

/// Byte-size gate for a downloaded volume: exact match or `Failed`.
/// Pure fs helper (no network), unit-tested on scratch files.
pub fn check_size(path: &Path, expected: u64) -> Result<(), String> {
    let actual = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "size mismatch for {}: got {actual} bytes, expected {expected}",
            path.display()
        ))
    }
}

/// Install check on an asset root (same rule as C#
/// `FileIO.IsSpotlightInstalled`, via [`Assets`](mappalachia_render::assets::Assets)):
/// any non-0-byte `.jpg` in any `img/spotlight` subfolder.
/// Pure fs query on fake trees in tests; the worker calls it to verify.
pub fn verify_install(assets_root: &Path) -> bool {
    mappalachia_render::assets::Assets::open(&assets_root.to_string_lossy())
        .is_spotlight_installed()
}

/// `Read + Seek` over two split-volume files as one virtual byte stream.
///
/// sevenz-rust only needs generic `Read + Seek` (it seeks to `End` once
/// for the length, then reads/seeks freely), so mapping a virtual cursor
/// across the `.001`→`.002` boundary is fully transparent to it — and
/// avoids a ~3 GB concat temp copy (spike report).
pub struct ChainedReader {
    first: File,
    second: File,
    first_len: u64,
    total_len: u64,
    pos: u64,
}

impl ChainedReader {
    /// Open both volumes; records lengths up front so `End`-relative
    /// seeks work without touching the files again.
    pub fn open(first_path: &Path, second_path: &Path) -> std::io::Result<Self> {
        let first = File::open(first_path)?;
        let first_len = first.metadata()?.len();
        let second = File::open(second_path)?;
        let total_len = first_len + second.metadata()?.len();
        Ok(Self {
            first,
            second,
            first_len,
            total_len,
            pos: 0,
        })
    }
}

impl Read for ChainedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.pos >= self.total_len {
            return Ok(0);
        }
        if self.pos < self.first_len {
            let cap = ((self.first_len - self.pos) as usize).min(buf.len());
            self.first.seek(SeekFrom::Start(self.pos))?;
            let n = self.first.read(&mut buf[..cap])?;
            self.pos += n as u64;
            // A short read inside the first volume is real EOF (the file
            // shrank under us); report it rather than crossing over.
            return Ok(n);
        }
        self.second
            .seek(SeekFrom::Start(self.pos - self.first_len))?;
        let n = self.second.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for ChainedReader {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let base = match from {
            SeekFrom::Start(offset) => offset as i128,
            SeekFrom::End(offset) => self.total_len as i128 + offset as i128,
            SeekFrom::Current(offset) => self.pos as i128 + offset as i128,
        };
        if base < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before start of chained volumes",
            ));
        }
        self.pos = base as u64;
        Ok(self.pos)
    }
}

/// Download one volume with throttled progress (pure streaming: a 64 KiB
/// buf, never the whole 2 GB in memory). Each `read` is bounded by
/// [`READ_TIMEOUT`]; there is no whole-download timeout.
fn download_one(
    client: &reqwest::blocking::Client,
    url: &str,
    dest: &Path,
    expected: u64,
    file_idx: usize,
    tx: &Sender<DlProgress>,
) -> Result<(), Abort> {
    let mut resp = client
        .get(url)
        .send()
        .map_err(|e| Abort::Failed(format!("download {file_idx}/2 failed: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(Abort::Failed(format!(
            "download {file_idx}/2 failed: HTTP {status}"
        )));
    }
    let total = resp.content_length().unwrap_or(expected);
    copy_body(&mut resp, dest, total, file_idx, tx)
}

/// Stream a response body to `dest` with throttled progress. A read error
/// — including [`READ_TIMEOUT`] firing on a stalled socket — surfaces as
/// `Failed` promptly instead of lingering; a dead receiver surfaces as
/// `Cancelled`. Takes any `Read` so tests can drive it with local fakes
/// (no network in tests).
fn copy_body(
    body: &mut dyn Read,
    dest: &Path,
    total: u64,
    file_idx: usize,
    tx: &Sender<DlProgress>,
) -> Result<(), Abort> {
    let mut out = File::create(dest)
        .map_err(|e| Abort::Failed(format!("cannot write {}: {e}", dest.display())))?;
    send(
        tx,
        DlProgress::Downloading {
            file_idx,
            bytes_done: 0,
            bytes_total: total,
        },
    )?;
    let mut buf = [0u8; 65536];
    let (mut done, mut last_sent) = (0u64, 0u64);
    loop {
        let n = body
            .read(&mut buf)
            .map_err(|e| Abort::Failed(format!("download {file_idx}/2 failed: {e}")))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])
            .map_err(|e| Abort::Failed(format!("cannot write {}: {e}", dest.display())))?;
        done += n as u64;
        if done - last_sent >= PROGRESS_EVERY_BYTES {
            last_sent = done;
            send(
                tx,
                DlProgress::Downloading {
                    file_idx,
                    bytes_done: done,
                    bytes_total: total,
                },
            )?;
        }
    }
    send(
        tx,
        DlProgress::Downloading {
            file_idx,
            bytes_done: done,
            bytes_total: total,
        },
    )?;
    Ok(())
}

/// Worker body: download → size-check → stage-extract → verify → promote,
/// then a terminal `Done`/`Failed`. Runs on a worker thread; only
/// `DlProgress` crosses threads. Always cleans the temp and staging dirs
/// before returning (a mid-download cancel leaves cleanup to the shared
/// helpers — both run them, and double-clean is harmless).
pub fn run(assets_root: &Path, tx: Sender<DlProgress>) {
    let outcome = run_inner(assets_root, &tx);
    clean_dl_dir(assets_root);
    clean_staging_dir(assets_root);
    match outcome {
        Ok(()) => {
            let _ = tx.send(DlProgress::Done);
        }
        Err(Abort::Failed(message)) => {
            let _ = tx.send(DlProgress::Failed(message));
        }
        Err(Abort::Cancelled) => {}
    }
}

fn run_inner(assets_root: &Path, tx: &Sender<DlProgress>) -> Result<(), Abort> {
    let staging = staging_dir(assets_root);
    let dir = dl_dir(assets_root);
    std::fs::create_dir_all(&dir)
        .map_err(|e| Abort::Failed(format!("cannot create {}: {e}", dir.display())))?;
    // No resume (YAGNI): a stale staging tree from a previous run is
    // dropped, not continued — extraction restarts from scratch.
    clean_staging_dir(assets_root);
    std::fs::create_dir_all(&staging)
        .map_err(|e| Abort::Failed(format!("cannot create {}: {e}", staging.display())))?;
    let client = build_client().map_err(Abort::Failed)?;
    let volumes = [
        (SPOTLIGHT_URL_001, SPOTLIGHT_FILE_001, SPOTLIGHT_SIZE_001),
        (SPOTLIGHT_URL_002, SPOTLIGHT_FILE_002, SPOTLIGHT_SIZE_002),
    ];
    for (i, (url, name, expected)) in volumes.iter().enumerate() {
        let dest = dir.join(name);
        // No resume (YAGNI): stale partials are truncated, not appended.
        download_one(&client, url, &dest, *expected, i + 1, tx)?;
        check_size(&dest, *expected).map_err(Abort::Failed)?;
    }
    extract_volumes(
        &dir.join(SPOTLIGHT_FILE_001),
        &dir.join(SPOTLIGHT_FILE_002),
        &staging,
        tx,
    )?;
    send(tx, DlProgress::Verifying).ok();
    // The staged payload must satisfy the install rule BEFORE it is
    // allowed near the live tree — a corrupt/empty extract fails here
    // with the live tree untouched.
    if !spotlight_tree_has_tiles(&staging.join("spotlight")) {
        return Err(Abort::Failed(
            "extraction finished but no spotlight tiles were found; \
             the archive may be corrupt — try downloading again"
                .to_string(),
        ));
    }
    promote_staging(assets_root).map_err(Abort::Failed)?;
    if !verify_install(assets_root) {
        return Err(Abort::Failed(
            "extraction finished but no spotlight tiles were found; \
             the archive may be corrupt — try downloading again"
                .to_string(),
        ));
    }
    Ok(())
}

/// Same rule as C# `FileIO.IsSpotlightInstalled` (and
/// [`Assets::is_spotlight_installed`](mappalachia_render::assets::Assets::is_spotlight_installed)),
/// but rooted at a given `spotlight/` dir instead of an asset root: any
/// non-0-byte `.jpg` in any subfolder. Used to gate the staged payload
/// before promotion; pure fs query on fake trees in tests.
fn spotlight_tree_has_tiles(spotlight_dir: &Path) -> bool {
    let Ok(subs) = std::fs::read_dir(spotlight_dir) else {
        return false;
    };
    for sub in subs.filter_map(|e| e.ok()) {
        if !sub.path().is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(sub.path()) else {
            continue;
        };
        for file in files.filter_map(|e| e.ok()) {
            let is_jpg = file
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg"));
            let is_file = file.file_type().is_ok_and(|t| t.is_file());
            if is_jpg && is_file && file.metadata().is_ok_and(|m| m.len() > 0) {
                return true;
            }
        }
    }
    false
}

/// Rename the verified staged payload (`<staging>/spotlight/`) into the
/// live loader path (`<assets>/img/spotlight/`). Same parent dir, so same
/// filesystem and an atomic rename; any previous tree (a good install
/// being re-downloaded, or leftovers) is removed first. Leaves the staging
/// dir gone on success; on any error the live tree is either untouched
/// (payload missing) or holds only the previous tree.
fn promote_staging(assets_root: &Path) -> Result<(), String> {
    let staged = staging_dir(assets_root).join("spotlight");
    if !staged.is_dir() {
        return Err(format!(
            "extraction produced no {} payload; the archive may be corrupt — \
             try downloading again",
            staged.display()
        ));
    }
    let live = assets_root.join("img").join("spotlight");
    if live.is_dir() && !live.is_symlink() {
        std::fs::remove_dir_all(&live)
            .map_err(|e| format!("cannot replace {}: {e}", live.display()))?;
    } else if std::fs::symlink_metadata(&live).is_ok() {
        std::fs::remove_file(&live)
            .map_err(|e| format!("cannot replace {}: {e}", live.display()))?;
    }
    std::fs::rename(&staged, &live)
        .map_err(|e| format!("cannot install {}: {e}", live.display()))?;
    clean_staging_dir(assets_root);
    Ok(())
}

/// Extract the downloaded pair into `dest_dir` (the archive's top-level
/// `spotlight/` lands directly under it — pass the staging dir, never the
/// live `img/` tree). Reports per-file progress; directory entries are
/// created but not counted.
fn extract_volumes(
    first: &Path,
    second: &Path,
    dest_dir: &Path,
    tx: &Sender<DlProgress>,
) -> Result<(), Abort> {
    let reader = ChainedReader::open(first, second)
        .map_err(|e| Abort::Failed(format!("cannot read downloads: {e}")))?;
    send(
        tx,
        DlProgress::Extracting {
            done: 0,
            total: EXPECTED_FILE_COUNT,
        },
    )?;
    let mut done = 0usize;
    sevenz_rust::decompress_with_extract_fn(reader, dest_dir, |entry, reader, dest| {
        let keep = sevenz_rust::default_entry_extract_fn(entry, reader, dest)
            .map_err(|e| sevenz_rust::Error::other(format!("extract failed: {e}")))?;
        if keep && !entry.is_directory() {
            done += 1;
            if tx
                .send(DlProgress::Extracting {
                    done,
                    total: EXPECTED_FILE_COUNT,
                })
                .is_err()
            {
                return Err(sevenz_rust::Error::other("cancelled"));
            }
        }
        Ok(keep)
    })
    .map_err(|e| {
        if e.to_string() == "cancelled" {
            Abort::Cancelled
        } else {
            Abort::Failed(format!("extract failed: {e}"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_applies_every_progress_event() {
        let mut state = DlState::Idle;
        assert!(!state.is_active());
        state.apply(DlProgress::Downloading {
            file_idx: 1,
            bytes_done: 10,
            bytes_total: 100,
        });
        assert_eq!(
            state,
            DlState::Downloading {
                file_idx: 1,
                bytes_done: 10,
                bytes_total: 100
            }
        );
        assert!(state.is_active());
        state.apply(DlProgress::Downloading {
            file_idx: 2,
            bytes_done: 0,
            bytes_total: 50,
        });
        assert_eq!(
            state,
            DlState::Downloading {
                file_idx: 2,
                bytes_done: 0,
                bytes_total: 50
            }
        );
        state.apply(DlProgress::Extracting {
            done: 3,
            total: 466,
        });
        assert_eq!(
            state,
            DlState::Extracting {
                done: 3,
                total: 466
            }
        );
        assert!(state.is_active());
        state.apply(DlProgress::Verifying);
        assert_eq!(state, DlState::Verifying);
        assert!(state.is_active());
        state.apply(DlProgress::Done);
        assert_eq!(state, DlState::Done);
        assert!(!state.is_active());
    }

    #[test]
    fn state_failure_sticks() {
        let mut state = DlState::Downloading {
            file_idx: 1,
            bytes_done: 5,
            bytes_total: 10,
        };
        state.apply(DlProgress::Failed("boom".to_string()));
        assert_eq!(state, DlState::Failed("boom".to_string()));
        assert!(!state.is_active());
    }

    /// Scratch pair of plain files standing in for the two 7z volumes.
    fn chained_pair(tag: &str, first: &[u8], second: &[u8]) -> (PathBuf, PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("mappalachia-chain-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp pair dir");
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        std::fs::write(&a, first).expect("tmp first");
        std::fs::write(&b, second).expect("tmp second");
        (dir, a, b)
    }

    #[test]
    fn chained_reader_spans_both_files() {
        let (dir, a, b) = chained_pair("span", b"ABC", b"DEF");
        let mut reader = ChainedReader::open(&a, &b).expect("open pair");
        let mut all = Vec::new();
        reader.read_to_end(&mut all).expect("read all");
        assert_eq!(all, b"ABCDEF");
        remove_tmp(&dir);
    }

    #[test]
    fn chained_reader_seeks_across_boundary() {
        let (dir, a, b) = chained_pair("seek", b"AB", b"CDE");
        let mut reader = ChainedReader::open(&a, &b).expect("open pair");
        // Total length via End-seek (sevenz-rust does exactly this first).
        let total = reader.seek(SeekFrom::End(0)).expect("seek end");
        assert_eq!(total, 5);
        // Start inside the second volume, read across nothing.
        reader.seek(SeekFrom::Start(3)).expect("seek 3");
        let mut one = [0u8; 2];
        reader.read_exact(&mut one).expect("read at 3");
        assert_eq!(&one, b"DE");
        // Back into the first volume, straddling the boundary.
        reader.seek(SeekFrom::Start(1)).expect("seek 1");
        let mut two = [0u8; 3];
        reader.read_exact(&mut two).expect("read at 1");
        assert_eq!(&two, b"BCD");
        // Relative seek + position query.
        reader.seek(SeekFrom::Current(-4)).expect("seek back");
        assert_eq!(reader.stream_position().expect("pos"), 0);
        // Seek before start is an error, never wraps.
        assert!(reader.seek(SeekFrom::Current(-1)).is_err());
        // Past-end reads are clean EOF.
        reader.seek(SeekFrom::Start(99)).expect("seek past end");
        let mut buf = [0u8; 4];
        assert_eq!(reader.read(&mut buf).expect("past-end read"), 0);
        remove_tmp(&dir);
    }

    #[test]
    fn size_check_accepts_exact_rejects_mismatch() {
        let dir = std::env::temp_dir().join(format!("mappalachia-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp size dir");
        let file = dir.join("vol.bin");
        std::fs::write(&file, [7u8; 32]).expect("tmp vol");
        assert!(check_size(&file, 32).is_ok());
        let err = check_size(&file, 33).expect_err("mismatch must fail");
        assert!(err.contains("size mismatch"), "got: {err}");
        assert!(check_size(&dir.join("missing.bin"), 0).is_err());
        remove_tmp(&dir);
    }

    #[test]
    fn temp_cleanup_is_best_effort() {
        let root = std::env::temp_dir().join(format!("mappalachia-dlclean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Missing dir: no panic, no error.
        clean_dl_dir(&root);
        let dir = dl_dir(&root);
        std::fs::create_dir_all(&dir).expect("tmp dl dir");
        std::fs::write(dir.join(SPOTLIGHT_FILE_001), [1u8; 8]).expect("tmp partial");
        clean_dl_dir(&root);
        assert!(!dir.exists(), "temp dir must be gone");
        // Double-clean is harmless.
        clean_dl_dir(&root);
        remove_tmp(&root);
    }

    #[test]
    fn verify_matches_install_rule_on_fake_trees() {
        // Tree with one non-empty tile: installed.
        let root = std::env::temp_dir().join(format!("mappalachia-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sub = root.join("img/spotlight/APPALACHIA");
        std::fs::create_dir_all(&sub).expect("tmp verify tree");
        std::fs::write(sub.join("0.0.jpg"), [1u8; 16]).expect("tmp tile");
        assert!(verify_install(&root));
        // Zero-byte tile does not count (mirrors the C# non-0-byte rule).
        std::fs::write(sub.join("0.0.jpg"), []).expect("truncate tile");
        assert!(!verify_install(&root));
        // Missing tree entirely: not installed, never panics.
        remove_tmp(&root);
        assert!(!verify_install(&root));
    }

    #[test]
    fn client_builds_with_timeouts() {
        // No network: just proves the shared builder (connect + per-read
        // timeouts, no whole-download timeout) constructs.
        assert!(build_client().is_ok());
    }

    #[test]
    fn staging_cleanup_is_best_effort() {
        let root =
            std::env::temp_dir().join(format!("mappalachia-stageclean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Missing dir: no panic, no error.
        clean_staging_dir(&root);
        let dir = staging_dir(&root);
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some(STAGING_DIR_NAME)
        );
        std::fs::create_dir_all(dir.join("spotlight/SUB")).expect("tmp staging tree");
        std::fs::write(dir.join("spotlight/SUB/0.0.jpg"), [1u8; 8]).expect("tmp partial");
        clean_staging_dir(&root);
        assert!(!dir.exists(), "staging dir must be gone");
        // Double-clean is harmless.
        clean_staging_dir(&root);
        remove_tmp(&root);
    }

    #[test]
    fn staged_tree_gate_matches_install_rule() {
        let root =
            std::env::temp_dir().join(format!("mappalachia-stagegate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Missing payload: not installable, never panics.
        assert!(!spotlight_tree_has_tiles(
            &staging_dir(&root).join("spotlight")
        ));
        let sub = staging_dir(&root).join("spotlight/APPALACHIA");
        std::fs::create_dir_all(&sub).expect("tmp staged tree");
        // Empty dir alone does not count.
        assert!(!spotlight_tree_has_tiles(
            &staging_dir(&root).join("spotlight")
        ));
        // One non-empty tile: installable.
        std::fs::write(sub.join("0.0.jpg"), [1u8; 16]).expect("tmp staged tile");
        assert!(spotlight_tree_has_tiles(
            &staging_dir(&root).join("spotlight")
        ));
        // Zero-byte tile does not count (mirrors the C# non-0-byte rule).
        std::fs::write(sub.join("0.0.jpg"), []).expect("truncate staged tile");
        assert!(!spotlight_tree_has_tiles(
            &staging_dir(&root).join("spotlight")
        ));
        remove_tmp(&root);
    }

    #[test]
    fn promote_swaps_verified_payload_into_place() {
        let root = std::env::temp_dir().join(format!("mappalachia-promote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // A good previous install being re-downloaded.
        let live_sub = root.join("img/spotlight/OLD");
        std::fs::create_dir_all(&live_sub).expect("tmp live tree");
        std::fs::write(live_sub.join("9.9.jpg"), [7u8; 16]).expect("tmp live tile");
        // The verified staged payload.
        let staged_sub = staging_dir(&root).join("spotlight/APPALACHIA");
        std::fs::create_dir_all(&staged_sub).expect("tmp staged tree");
        std::fs::write(staged_sub.join("0.0.jpg"), [1u8; 16]).expect("tmp staged tile");
        promote_staging(&root).expect("promote");
        // Live tree now holds the new payload, not the old one ...
        assert!(!live_sub.exists(), "previous tree must be replaced");
        assert_eq!(
            std::fs::read(root.join("img/spotlight/APPALACHIA/0.0.jpg")).expect("new tile"),
            vec![1u8; 16]
        );
        // ... the staging dir is gone, and the live probe agrees.
        assert!(!staging_dir(&root).exists(), "staging dir must be gone");
        assert!(verify_install(&root));
        remove_tmp(&root);
    }

    #[test]
    fn promote_rejects_missing_payload_and_keeps_live_tree() {
        let root = std::env::temp_dir().join(format!(
            "mappalachia-promote-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let live_sub = root.join("img/spotlight/APPALACHIA");
        std::fs::create_dir_all(&live_sub).expect("tmp live tree");
        std::fs::write(live_sub.join("0.0.jpg"), [7u8; 16]).expect("tmp live tile");
        // Empty staging (corrupt/empty extract): promote refuses ...
        std::fs::create_dir_all(staging_dir(&root)).expect("tmp empty staging");
        assert!(promote_staging(&root).is_err());
        // ... and the previous good install is untouched.
        assert_eq!(
            std::fs::read(live_sub.join("0.0.jpg")).expect("live tile"),
            vec![7u8; 16]
        );
        assert!(verify_install(&root));
        remove_tmp(&root);
    }

    #[test]
    fn partial_extract_never_touches_live_tree() {
        // Finding-1 regression: a cancelled/failed mid-extract run leaves
        // partial tiles ONLY in staging; the live probe cannot misreport.
        let root = std::env::temp_dir().join(format!(
            "mappalachia-staging-partial-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("img")).expect("tmp img dir");
        // Simulate partial extraction output landing in staging.
        let partial = staging_dir(&root).join("spotlight/APPALACHIA");
        std::fs::create_dir_all(&partial).expect("tmp partial tree");
        std::fs::write(partial.join("0.0.jpg"), [1u8; 16]).expect("tmp partial tile");
        // Live tree has no partials, so the probe still reports missing.
        assert!(!root.join("img/spotlight").exists());
        assert!(!verify_install(&root));
        // Cancel/fail cleanup removes staging; the live tree is untouched.
        clean_staging_dir(&root);
        assert!(!staging_dir(&root).exists());
        assert!(!root.join("img/spotlight").exists());
        assert!(!verify_install(&root));
        remove_tmp(&root);
    }

    /// Fake body: yields `head` bytes once, then fails the way a socket
    /// read does when the per-read timeout fires on a stalled connection.
    struct StalledBody {
        head: Vec<u8>,
        spent: bool,
    }

    impl Read for StalledBody {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.spent {
                self.spent = true;
                let n = self.head.len().min(buf.len());
                buf[..n].copy_from_slice(&self.head[..n]);
                return Ok(n);
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "simulated stalled read",
            ))
        }
    }

    #[test]
    fn copy_body_streams_with_progress() {
        let dir = std::env::temp_dir().join(format!("mappalachia-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp copy dir");
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mut body = std::io::Cursor::new(data.clone());
        let (tx, rx) = std::sync::mpsc::channel::<DlProgress>();
        let dest = dir.join("vol.bin");
        copy_body(&mut body, &dest, data.len() as u64, 1, &tx).expect("stream");
        assert_eq!(std::fs::read(&dest).expect("written"), data);
        let events: Vec<DlProgress> = rx.try_iter().collect();
        assert!(
            events.len() >= 2,
            "initial + final sends, got {}",
            events.len()
        );
        assert_eq!(
            events[0],
            DlProgress::Downloading {
                file_idx: 1,
                bytes_done: 0,
                bytes_total: data.len() as u64
            }
        );
        assert_eq!(
            events.last(),
            Some(&DlProgress::Downloading {
                file_idx: 1,
                bytes_done: data.len() as u64,
                bytes_total: data.len() as u64
            })
        );
        remove_tmp(&dir);
    }

    #[test]
    fn copy_body_surfaces_stalled_read_as_failure() {
        // A stalled socket (read timeout fired) must surface as `Failed`
        // promptly — never linger, never succeed with a partial file.
        let dir =
            std::env::temp_dir().join(format!("mappalachia-copy-stall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp stall dir");
        let mut body = StalledBody {
            head: vec![1u8; 64],
            spent: false,
        };
        let (tx, _rx) = std::sync::mpsc::channel::<DlProgress>();
        let err = copy_body(&mut body, &dir.join("vol.bin"), 1024, 2, &tx)
            .expect_err("stalled read must fail");
        match err {
            Abort::Failed(message) => assert!(message.contains("failed"), "got: {message}"),
            Abort::Cancelled => panic!("a live receiver must not read as cancel"),
        }
        remove_tmp(&dir);
    }

    #[test]
    fn copy_body_cancel_is_quiet() {
        // Dropped receiver (user cancelled): the first progress send fails
        // and the copy aborts as `Cancelled`, not `Failed`.
        let dir =
            std::env::temp_dir().join(format!("mappalachia-copy-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmp cancel dir");
        let (tx, rx) = std::sync::mpsc::channel::<DlProgress>();
        drop(rx);
        let mut body = std::io::Cursor::new(vec![1u8; 64]);
        let err = copy_body(&mut body, &dir.join("vol.bin"), 64, 1, &tx)
            .expect_err("dead receiver must abort");
        assert!(matches!(err, Abort::Cancelled));
        remove_tmp(&dir);
    }

    fn remove_tmp(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }
}
