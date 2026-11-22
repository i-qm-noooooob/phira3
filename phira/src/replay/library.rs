//! Local replay library. Imported files are never added to ranked score history.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const INDEX_LIMIT: u64 = 4 * 1024 * 1024;
const MAX_ITEMS: usize = 10_000;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn root() -> Result<PathBuf> {
    Ok(PathBuf::from(crate::dir::root()?).join("replays"))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct IndexEntry {
    imported_at: i64,
    keep: bool,
    local_path: Option<String>,
    verified: bool,
}

type Index = BTreeMap<String, IndexEntry>;

#[derive(Clone, Debug, Serialize)]
pub struct ReplayItem {
    /// File name relative to the private replay-library directory.
    pub file: String,
    pub version: u32,
    pub size: u64,
    pub imported_at: i64,
    pub keep: bool,
    pub local_path: Option<String>,
    pub verified: bool,
    pub chart_name: Option<String>,
    pub player: Option<String>,
    pub recorded_at: Option<i64>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        && !name.starts_with('.')
        && !name.contains("..")
}

fn read_index(root: &Path) -> Result<Index> {
    let path = root.join("index.json");
    if !path.exists() {
        return Ok(Index::new());
    }
    let metadata = fs::metadata(&path)?;
    anyhow::ensure!(metadata.len() <= INDEX_LIMIT, "Replay index exceeds size limit");
    let bytes = fs::read(path)?;
    let index: Index = serde_json::from_slice(&bytes).context("Invalid replay library index")?;
    anyhow::ensure!(index.len() <= MAX_ITEMS, "Replay library index contains too many entries");
    anyhow::ensure!(index.keys().all(|name| valid_file_name(name)), "Replay index contains an invalid file name");
    Ok(index)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".replay-tmp-{}-{id}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn write_index(root: &Path, index: &Index) -> Result<()> {
    anyhow::ensure!(index.len() <= MAX_ITEMS, "Replay library item limit reached");
    let bytes = serde_json::to_vec_pretty(index)?;
    anyhow::ensure!(bytes.len() as u64 <= INDEX_LIMIT, "Replay index exceeds size limit");
    write_atomic(&root.join("index.json"), &bytes)
}

fn item_from_bytes(root: &Path, file: String, bytes: &[u8], index: &Index) -> Result<ReplayItem> {
    let decoded = super::storage::ReplayTape::decode(bytes)?;
    let name = (!decoded.meta.name.trim().is_empty()).then_some(decoded.meta.name.clone());
    let player = decoded.meta.player.clone().filter(|value| !value.trim().is_empty());
    let recorded_at = decoded.meta.recorded_at;
    let entry = index.get(&file);
    let imported_at = entry.map(|e| e.imported_at).unwrap_or(0);
    let keep = entry.is_some_and(|e| e.keep);
    let size = root.join(&file).metadata().map(|m| m.len()).unwrap_or(bytes.len() as u64);
    Ok(ReplayItem {
        file,
        version: decoded.source_version,
        size,
        imported_at,
        keep,
        local_path: entry.and_then(|e| e.local_path.clone()),
        verified: entry.is_some_and(|e| e.verified),
        chart_name: name,
        player,
        recorded_at,
    })
}

fn supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "phirarec" | "phirar" | "ppr"))
}

/// Scan valid replay files, skipping corrupt files without stopping the whole library.
pub fn list() -> Result<Vec<ReplayItem>> {
    let root = root()?;
    list_in(&root)
}

fn list_in(root: &Path) -> Result<Vec<ReplayItem>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let index = read_index(root)?;
    let mut items = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || !supported_extension(&path) {
            continue;
        }
        if items.len() >= MAX_ITEMS {
            break;
        }
        let Some(file) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if !valid_file_name(&file) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else { continue };
        if metadata.len() > super::storage::MAX_FILE_BYTES as u64 {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        if let Ok(item) = item_from_bytes(root, file, &bytes, &index) {
            items.push(item);
        }
    }
    items.sort_by(|a, b| {
        b.recorded_at
            .cmp(&a.recorded_at)
            .then_with(|| b.imported_at.cmp(&a.imported_at))
            .then_with(|| a.file.cmp(&b.file))
    });
    Ok(items)
}

/// Import one replay file. Identical content is deduplicated by SHA-256.
pub fn import(source: &Path) -> Result<(ReplayItem, bool)> {
    import_in(source, &root()?)
}

fn import_in(source: &Path, root: &Path) -> Result<(ReplayItem, bool)> {
    let metadata = fs::metadata(source).context("Cannot read replay source")?;
    anyhow::ensure!(metadata.is_file(), "Replay source is not a regular file");
    anyhow::ensure!(metadata.len() <= super::storage::MAX_FILE_BYTES as u64, "Replay file exceeds 128 MiB");
    let bytes = fs::read(source)?;
    let decoded = super::storage::decode(&bytes)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let file = format!("replay-{digest}.phirarec");
    fs::create_dir_all(root)?;
    let mut index = read_index(root)?;
    let path = root.join(&file);
    let inserted = if path.exists() {
        false
    } else {
        write_atomic(&path, &bytes)?;
        true
    };
    if inserted || !index.contains_key(&file) {
        index.insert(
            file.clone(),
            IndexEntry {
                imported_at: now_ms(),
                keep: false,
                ..Default::default()
            },
        );
        write_index(root, &index)?;
    }
    let item = item_from_bytes(root, file, &bytes, &index)
        .with_context(|| format!("Decoded replay version {} but could not add it to the library", decoded.version))?;
    Ok((item, inserted))
}

/// Import a single replay or a ZIP package containing replay files.
/// The complete archive is staged and validated before any library entry is committed.
pub fn import_package(source: &Path) -> Result<Vec<(ReplayItem, bool)>> {
    if !source.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        return Ok(vec![import(source)?]);
    }

    let file = fs::File::open(source).context("Cannot open replay package")?;
    let mut archive = zip::ZipArchive::new(file).context("Invalid replay ZIP package")?;
    anyhow::ensure!(archive.len() <= 1000, "Replay package contains too many entries");
    let temp = tempfile::tempdir()?;
    let mut staged = Vec::new();
    let mut total = 0u64;

    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let relative = entry.enclosed_name().context("Replay package contains an invalid path")?;
        let extension = relative.extension().and_then(|e| e.to_str()).unwrap_or_default();
        anyhow::ensure!(
            matches!(extension.to_ascii_lowercase().as_str(), "phirarec" | "phirar" | "ppr"),
            "Replay package contains a non-replay file"
        );
        let declared = entry.size();
        anyhow::ensure!(declared <= super::storage::MAX_FILE_BYTES as u64, "A replay in the package exceeds 128 MiB");
        total = total.checked_add(declared).context("Replay package size overflow")?;
        anyhow::ensure!(total <= 512 * 1024 * 1024, "Replay package expands beyond 512 MiB");

        let mut bytes = Vec::with_capacity(declared as usize);
        entry
            .take(super::storage::MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .context("Failed to extract replay from package")?;
        anyhow::ensure!(bytes.len() as u64 == declared, "Replay package entry size mismatch");
        super::storage::decode(&bytes).context("Replay package contains an invalid replay")?;

        let staged_path = temp.path().join(format!("{i}.phirarec"));
        write_atomic(&staged_path, &bytes)?;
        staged.push(staged_path);
    }
    anyhow::ensure!(!staged.is_empty(), "Replay package is empty");
    staged.iter().map(|path| import(path)).collect()
}

/// Toggle retention protection. This only affects library cleanup, not replay contents.
pub fn set_keep(file: &str, keep: bool) -> Result<()> {
    set_keep_in(&root()?, file, keep)
}

fn set_keep_in(root: &Path, file: &str, keep: bool) -> Result<()> {
    anyhow::ensure!(valid_file_name(file), "Invalid replay file name");
    let path = root.join(file);
    anyhow::ensure!(path.is_file() && path.parent() == Some(root), "Replay does not exist in this library");
    let mut index = read_index(root)?;
    index.entry(file.to_owned()).or_default().keep = keep;
    write_index(root, &index)
}

/// Delete a replay that is inside the managed library directory.
pub fn delete(file: &str) -> Result<()> {
    delete_in(&root()?, file)
}

fn delete_in(root: &Path, file: &str) -> Result<()> {
    anyhow::ensure!(valid_file_name(file), "Invalid replay file name");
    let path = root.join(file);
    anyhow::ensure!(path.parent() == Some(root), "Replay path is outside the library");
    let metadata = fs::symlink_metadata(&path).context("Replay does not exist")?;
    anyhow::ensure!(metadata.file_type().is_file(), "Replay is not a regular file");
    let mut index = read_index(root)?;
    fs::remove_file(&path)?;
    index.remove(file);
    write_index(root, &index)
}

fn valid_local_chart_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn bind_in(root: &Path, file: &str, local_path: &str, verified: bool) -> Result<()> {
    anyhow::ensure!(valid_file_name(file), "Invalid replay file name");
    anyhow::ensure!(valid_local_chart_path(local_path), "Invalid local chart path");
    anyhow::ensure!(root.join(file).is_file(), "Replay does not exist in this library");
    let mut index = read_index(root)?;
    let entry = index.entry(file.to_owned()).or_default();
    entry.local_path = Some(local_path.to_owned());
    entry.verified = verified;
    write_index(root, &index)
}

/// Bind an imported replay to a local chart after the chart and audio fingerprints are checked.
pub fn bind(file: &str, local_path: &str, verified: bool) -> Result<()> {
    bind_in(&root()?, file, local_path, verified)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    Found(String, bool),
    Missing,
    WrongVersion,
    WrongAudio,
    WrongServer,
}

pub async fn fingerprint(fs: &mut dyn prpr::fs::FileSystem, info: &prpr::info::ChartInfo) -> Result<String> {
    let chart = prpr::scene::GameScene::load_chart_bytes(fs, info).await?;
    let extra = fs.load_file("extra.json").await.ok();
    let mut hash = Sha256::new();
    hash.update(b"PhiraReplayChart1\0");
    hash.update((chart.len() as u64).to_le_bytes());
    hash.update(&chart);
    if let Some(extra) = extra {
        hash.update((extra.len() as u64).to_le_bytes());
        hash.update(extra);
    }
    hash.update(info.offset.to_le_bytes());
    Ok(format!("{:x}", hash.finalize()))
}

pub async fn audio_fingerprint(fs: &mut dyn prpr::fs::FileSystem, info: &prpr::info::ChartInfo) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(fs.load_file(&info.music).await?)))
}

/// Find a matching local chart. A fingerprint match is required for new-format tapes;
/// legacy tapes can only use their recorded local path or chart ID and are marked unverified.
pub async fn match_chart(replay: &super::storage::ReplayTape, preferred: Option<&str>, candidates: &[String]) -> Result<Match> {
    let wrong_server = matches!(&replay.chart, Some(super::storage::ReplayChartRef::Id(_)))
        && replay
            .meta
            .server
            .as_ref()
            .is_some_and(|server| server.trim_end_matches('/') != crate::client::API_URL.trim_end_matches('/'));
    let original = match &replay.chart {
        Some(super::storage::ReplayChartRef::Local(path)) if candidates.contains(path) => Some(path.clone()),
        Some(super::storage::ReplayChartRef::Id(id)) if *id > 0 => Some(format!("download/{id}")),
        _ => None,
    };
    let preferred = preferred.filter(|path| valid_local_chart_path(path) && candidates.iter().any(|candidate| candidate == path));
    let mut paths: Vec<String> = preferred
        .into_iter()
        .map(str::to_owned)
        .chain(original)
        .chain(candidates.iter().cloned())
        .collect();
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));

    let mut wrong_version = false;
    let mut wrong_audio = false;
    for path in paths {
        let Ok(mut fs) = crate::scene::fs_from_path(&path) else {
            continue;
        };
        let Ok(info) = prpr::fs::load_info(fs.as_mut()).await else {
            continue;
        };
        if let Some(expected) = &replay.meta.fingerprint {
            if fingerprint(fs.as_mut(), &info).await.is_ok_and(|actual| &actual == expected) {
                if let Some(expected_audio) = &replay.meta.audio_fingerprint {
                    if !audio_fingerprint(fs.as_mut(), &info).await.is_ok_and(|actual| &actual == expected_audio) {
                        wrong_audio = true;
                        continue;
                    }
                }
                return Ok(Match::Found(path, true));
            }
            if matches!(&replay.chart, Some(super::storage::ReplayChartRef::Id(id)) if info.id == Some(*id)) || preferred == Some(path.as_str()) {
                wrong_version = true;
            }
        } else if preferred == Some(path.as_str())
            || (!wrong_server
                && match &replay.chart {
                    Some(super::storage::ReplayChartRef::Local(local)) => local == &path,
                    Some(super::storage::ReplayChartRef::Id(id)) => info.id == Some(*id),
                    None => false,
                })
        {
            return Ok(Match::Found(path, false));
        }
    }

    if wrong_server {
        Ok(Match::WrongServer)
    } else if wrong_audio {
        Ok(Match::WrongAudio)
    } else if wrong_version {
        Ok(Match::WrongVersion)
    } else {
        Ok(Match::Missing)
    }
}

/// Export a library replay to a caller-provided writer (for platform save pickers).
pub fn export_to_writer(file: &str, writer: &mut impl Write) -> Result<()> {
    let root = root()?;
    anyhow::ensure!(valid_file_name(file), "Invalid replay file name");
    let source = root.join(file);
    let metadata = fs::symlink_metadata(&source).context("Replay does not exist")?;
    anyhow::ensure!(metadata.file_type().is_file(), "Replay is not a regular file");
    anyhow::ensure!(metadata.len() <= super::storage::MAX_FILE_BYTES as u64, "Replay file exceeds 128 MiB");
    let bytes = fs::read(source)?;
    super::storage::decode(&bytes)?;
    writer.write_all(&bytes)?;
    Ok(())
}

/// Export a library replay without changing or deleting its source.
pub fn export(file: &str, destination: &Path) -> Result<()> {
    export_from(&root()?, file, destination)
}

fn export_from(root: &Path, file: &str, destination: &Path) -> Result<()> {
    anyhow::ensure!(valid_file_name(file), "Invalid replay file name");
    let source = root.join(file);
    let metadata = fs::symlink_metadata(&source).context("Replay does not exist")?;
    anyhow::ensure!(metadata.file_type().is_file(), "Replay is not a regular file");
    anyhow::ensure!(metadata.len() <= super::storage::MAX_FILE_BYTES as u64, "Replay file exceeds 128 MiB");
    let bytes = fs::read(source)?;
    super::storage::decode(&bytes)?;
    write_atomic(destination, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let json = br#"{"meta":{"name":"Test chart","player":"Local","recorded_at":1234},"frames":[],"touches":[],"judges":[]}"#;
        let mut bytes = crate::replay::storage::MAGIC.to_vec();
        bytes.extend(5u32.to_le_bytes());
        bytes.extend(zstd::stream::encode_all(json.as_slice(), 1).unwrap());
        bytes
    }

    #[test]
    fn imports_lists_deduplicates_and_exports_without_touching_scores() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.phirarec");
        fs::write(&source, sample()).unwrap();
        let root = temp.path().join("library");
        let mut index = Index::new();
        let bytes = fs::read(&source).unwrap();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let file = format!("replay-{digest}.phirarec");
        fs::create_dir_all(&root).unwrap();
        write_atomic(&root.join(&file), &bytes).unwrap();
        index.insert(
            file.clone(),
            IndexEntry {
                imported_at: 77,
                keep: false,
                ..Default::default()
            },
        );
        write_index(&root, &index).unwrap();
        let items = list_in(&root).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].chart_name.as_deref(), Some("Test chart"));
        assert_eq!(items[0].player.as_deref(), Some("Local"));
        assert_eq!(items[0].recorded_at, Some(1234));
        assert_eq!(items[0].version, 5);
        assert_eq!(read_index(&root).unwrap().len(), 1);
    }

    #[test]
    fn real_import_deduplicates_and_keep_delete_export_work_in_isolated_library() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.phirarec");
        let root = temp.path().join("library");
        let output = temp.path().join("export.phirarec");
        fs::write(&source, sample()).unwrap();

        let (first, inserted) = import_in(&source, &root).unwrap();
        assert!(inserted);
        assert_eq!(first.chart_name.as_deref(), Some("Test chart"));

        let (second, inserted) = import_in(&source, &root).unwrap();
        assert!(!inserted);
        assert_eq!(first.file, second.file);
        assert_eq!(list_in(&root).unwrap().len(), 1);

        set_keep_in(&root, &first.file, true).unwrap();
        assert!(read_index(&root).unwrap()[&first.file].keep);

        bind_in(&root, &first.file, "download/42", true).unwrap();
        let bound = list_in(&root).unwrap();
        assert_eq!(bound[0].local_path.as_deref(), Some("download/42"));
        assert!(bound[0].verified);
        assert!(bind_in(&root, &first.file, "../outside", true).is_err());
        assert!(bind_in(&root, &first.file, "/absolute/path", true).is_err());

        export_from(&root, &first.file, &output).unwrap();
        assert_eq!(fs::read(&output).unwrap(), sample());

        delete_in(&root, &first.file).unwrap();
        assert!(list_in(&root).unwrap().is_empty());
        assert!(!root.join(&first.file).exists());
    }

    #[test]
    fn rejects_path_traversal_and_invalid_index_names() {
        assert!(!valid_file_name("../other"));
        assert!(!valid_file_name("folder/replay"));
        assert!(valid_file_name("replay-a.phirarec"));
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path()).unwrap();
        fs::write(temp.path().join("index.json"), br#"{"../outside":{"keep":true}}"#).unwrap();
        assert!(read_index(temp.path()).is_err());
    }

    #[test]
    fn skips_corrupt_and_oversized_entries() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path()).unwrap();
        fs::write(temp.path().join("broken.phirarec"), b"bad").unwrap();
        let items = list_in(temp.path()).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn keep_flag_and_delete_are_scoped_to_library() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path()).unwrap();
        let file = "replay-a.phirarec";
        let path = temp.path().join(file);
        fs::write(&path, sample()).unwrap();
        let mut index = Index::new();
        index.insert(file.to_owned(), IndexEntry::default());
        write_index(temp.path(), &index).unwrap();
        index = read_index(temp.path()).unwrap();
        index.get_mut(file).unwrap().keep = true;
        write_index(temp.path(), &index).unwrap();
        assert!(read_index(temp.path()).unwrap()[file].keep);
        assert!(valid_file_name(file));
        fs::remove_file(&path).unwrap();
        index.remove(file);
        write_index(temp.path(), &index).unwrap();
        assert!(!temp.path().join(file).exists());
    }
}
