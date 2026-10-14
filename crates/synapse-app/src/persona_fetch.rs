//! Downloading the open agent collections (docs/PERFORMANCE.md: persona
//! collections are fetched on first use, not bundled).
//!
//! Each collection is one GitHub tarball. We keep only files that are real
//! agent definitions (frontmatter with a name and description, enough body),
//! cache them on disk as gzip JSON, and feed them to the persona library, so
//! later launches work offline.

use crate::persona_library::LibraryCache;
use synapse_personas::PersonaDraft;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;

/// A collection we download. All are MIT licensed.
pub struct Collection {
    pub repo: &'static str,
    /// Directory (relative to the repo root) -> category, or `None` to skip the file.
    category: fn(&str) -> Option<String>,
}

fn division(path: &str) -> Option<String> {
    const DIVISIONS: [&str; 17] = [
        "academic", "design", "engineering", "finance", "game-development", "gis", "healthcare",
        "marketing", "paid-media", "product", "project-management", "research", "sales",
        "security", "specialized", "support", "testing",
    ];
    let mut parts = path.split('/');
    let (first, second, third) = (parts.next()?, parts.next()?, parts.next());
    (third.is_none() && DIVISIONS.contains(&first) && second.ends_with(".md"))
        .then(|| first.to_string())
}

fn plugin(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (parts.len() == 4 && parts[0] == "plugins" && parts[2] == "agents" && parts[3].ends_with(".md"))
        .then(|| parts[1].to_string())
}

fn numbered_category(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (parts.len() == 3 && parts[0] == "categories" && parts[2].ends_with(".md")).then(|| {
        parts[1]
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start_matches('-')
            .to_string()
    })
}

fn agents_dir(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (parts.len() >= 2 && parts[0] == "agents" && path.ends_with(".md")).then(|| {
        if parts.len() > 2 {
            parts[parts.len() - 2].to_string()
        } else {
            "general".to_string()
        }
    })
}

/// Best-known open collections of well-written agents, broadest first.
pub const COLLECTIONS: [Collection; 4] = [
    Collection {
        repo: "msitarzewski/agency-agents",
        category: division,
    },
    Collection {
        repo: "wshobson/agents",
        category: plugin,
    },
    Collection {
        repo: "VoltAgent/awesome-claude-code-subagents",
        category: numbered_category,
    },
    Collection {
        repo: "lst97/claude-code-sub-agents",
        category: agents_dir,
    },
];

const MIN_BODY: usize = 1500;
const MAX_FILE: usize = 14_000;

/// Download one collection and keep its good agent files as `(path, markdown)`.
/// Paths are rewritten to `<category>/<file>.md` so the library groups them.
/// Runs on the tokio bridge, so unpacking never touches the UI thread.
pub fn download(
    collection: &'static Collection,
) -> crate::runtime::BridgeTask<Result<Vec<(String, String)>, String>> {
    crate::runtime::spawn(async move {
        let url = format!("https://codeload.github.com/{}/tar.gz/HEAD", collection.repo);
        let bytes = crate::runtime::fetch_bytes(url)
            .await
            .map_err(|e| e.to_string())??;
        Ok(select(collection, tar_gz_files(&bytes)?))
    })
}

/// Keep real agent definitions, drop the rest.
fn select(collection: &Collection, files: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (path, text) in files {
        let Some(category) = (collection.category)(&path) else {
            continue;
        };
        if text.len() > MAX_FILE || !is_agent(&text) {
            continue;
        }
        let file = path.rsplit('/').next().unwrap_or(&path);
        out.push((format!("{category}/{file}"), text));
    }
    out
}

/// Frontmatter with a `name` and `description`, then a body worth reading.
fn is_agent(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("---") else {
        return false;
    };
    let Some((front, body)) = rest.split_once("\n---") else {
        return false;
    };
    let has = |key: &str| {
        front
            .lines()
            .any(|l| l.trim_start().strip_prefix(key).is_some_and(|r| r.starts_with(':')))
    };
    has("name") && has("description") && body.trim().len() >= MIN_BODY
}

/// Regular files of a `.tar.gz` as `(path without the top folder, text)`.
/// Only what GitHub tarballs use: ustar headers, plus pax `path` records.
fn tar_gz_files(gz: &[u8]) -> Result<Vec<(String, String)>, String> {
    let mut data = Vec::new();
    GzDecoder::new(gz)
        .read_to_end(&mut data)
        .map_err(|e| format!("not a gzip archive: {e}"))?;
    let mut files = Vec::new();
    let mut pax_path: Option<String> = None;
    let mut at = 0;
    while at + 512 <= data.len() {
        let header = &data[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let field = |range: std::ops::Range<usize>| {
            let raw = &header[range];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).trim().to_string()
        };
        let size = usize::from_str_radix(&field(124..136), 8).map_err(|_| "bad tar header")?;
        let body_start = at + 512;
        let body_end = body_start.checked_add(size).filter(|&e| e <= data.len());
        let Some(body_end) = body_end else {
            return Err("truncated archive".into());
        };
        let body = &data[body_start..body_end];
        match header[156] {
            b'x' => {
                // pax record: "<len> path=<value>\n"
                let text = String::from_utf8_lossy(body);
                pax_path = text
                    .lines()
                    .find_map(|l| l.split_once(" path=").map(|(_, p)| p.to_string()));
            }
            b'0' | 0 => {
                let name = pax_path.take().unwrap_or_else(|| {
                    let prefix = field(345..500);
                    let name = field(0..100);
                    if prefix.is_empty() {
                        name
                    } else {
                        format!("{prefix}/{name}")
                    }
                });
                let path = name.split_once('/').map_or(name.as_str(), |(_, rest)| rest);
                if path.ends_with(".md") {
                    if let Ok(text) = std::str::from_utf8(body) {
                        files.push((path.to_string(), text.to_string()));
                    }
                }
            }
            _ => {}
        }
        at = body_start + size.div_ceil(512) * 512;
    }
    Ok(files)
}

fn cache_path(repo: &str) -> PathBuf {
    crate::app_dirs::data_dir()
        .join("personas")
        .join(format!("{}.json.gz", repo.replace('/', "_")))
}

/// Save a downloaded collection for later launches.
pub fn save_cached(repo: &str, files: &[(String, String)]) {
    let path = cache_path(repo);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(json) = serde_json::to_vec(files) else {
        return;
    };
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if encoder.write_all(&json).is_ok() {
        if let Ok(bytes) = encoder.finish() {
            let _ = std::fs::write(path, bytes);
        }
    }
}

fn load_cached(repo: &str) -> Option<Vec<(String, String)>> {
    let bytes = std::fs::read(cache_path(repo)).ok()?;
    let mut json = String::new();
    GzDecoder::new(&bytes[..]).read_to_string(&mut json).ok()?;
    serde_json::from_str(&json).ok()
}

/// Every collection already downloaded, parsed (call off the UI thread).
pub fn load_saved() -> Vec<PersonaDraft> {
    COLLECTIONS
        .iter()
        .filter_map(|c| load_cached(c.repo).map(|files| parse(c.repo, files)))
        .flatten()
        .collect()
}

/// Parse a collection's files into drafts (call off the UI thread).
pub fn parse(repo: &str, files: Vec<(String, String)>) -> Vec<PersonaDraft> {
    let mut cache = LibraryCache::new();
    cache.insert_tarball(repo, "downloaded", files, "MIT");
    cache.entries_for(repo).to_vec()
}

/// Collections not downloaded yet.
pub fn missing() -> Vec<&'static Collection> {
    COLLECTIONS
        .iter()
        .filter(|c| !cache_path(c.repo).exists())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_entry(name: &str, body: &str) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        let size = format!("{:011o}", body.len());
        header[124..135].copy_from_slice(size.as_bytes());
        header[156] = b'0';
        let mut out = header;
        out.extend_from_slice(body.as_bytes());
        out.resize(out.len().div_ceil(512) * 512, 0);
        out
    }

    #[test]
    fn reads_markdown_files_from_a_tarball_and_keeps_only_real_agents() {
        let agent = format!(
            "---\nname: Security Auditor\ndescription: Finds holes.\n---\n{}",
            "Audit carefully. ".repeat(120)
        );
        let mut tar = tar_entry("repo-HEAD/security/security-auditor.md", &agent);
        tar.extend(tar_entry("repo-HEAD/security/README.md", "# not an agent"));
        tar.extend(tar_entry("repo-HEAD/docs/guide.md", &agent));
        tar.extend(vec![0u8; 1024]);
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar).unwrap();
        let gz = gz.finish().unwrap();

        let files = tar_gz_files(&gz).unwrap();
        assert_eq!(files.len(), 3);
        let kept = select(&COLLECTIONS[0], files);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0, "security/security-auditor.md");
    }

    /// Real downloads; run with `cargo test -p synapse-app live_collections -- --ignored`.
    #[test]
    #[ignore = "needs network"]
    fn live_collections_download_and_parse() {
        for collection in &COLLECTIONS {
            let files = crate::runtime::block_on(download(collection))
                .expect("task")
                .expect("download");
            let mut cache = LibraryCache::new();
            cache.insert_tarball(collection.repo, "test", files.clone(), "MIT");
            println!("{}: {} files, {} parsed", collection.repo, files.len(), cache.entries_for(collection.repo).len());
            assert!(!files.is_empty(), "{} gave no agents", collection.repo);
        }
    }
}
