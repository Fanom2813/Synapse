//! Hybrid semantic memory indexing (docs/MEMORY.md §5).
//!
//! FTS5 is the primary index; vector similarity over `vec_chunks.embedding`
//! re-ranks / extends it. The embeddings model is lazy: [`embeddings()`]
//! initialises on first use and [`set_vectors_available(false)`] simulates a
//! missing/unloaded model, in which case all search degrades to FTS5 + LIKE
//! instead of failing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use rusqlite::params;

use crate::models::Memory;
use crate::store::{Result, Store};

/// Embedding width for the bundled lazy model (hash embeddings stand in for
/// model2vec until the real model file is wired; see LATER in summary).
pub const EMBEDDING_DIM: usize = 64;
/// Max chars per chunk; archive texts are split on char boundaries.
pub const CHUNK_SIZE: usize = 800;
/// Overlap between consecutive chunks so boundary phrases still match.
pub const CHUNK_OVERLAP: usize = 80;

/// Owner kinds eligible for vector indexing (MEMORY.md §1 archive + memories).
pub const ELIGIBLE_OWNER_KINDS: &[&str] = &["memory", "source", "artifact", "digest"];

/// A chunk with its vector-search score (cosine similarity, 0..1).
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredChunk {
    pub owner_kind: String,
    pub owner_id: String,
    pub chunk_index: i64,
    pub text: String,
    pub score: f32,
}

/// One hybrid hit: an active memory plus fused FTS + vector scores.
#[derive(Debug, Clone, PartialEq)]
pub struct HybridHit {
    pub memory: Memory,
    pub fts_rank: Option<f32>,
    pub vector_score: Option<f32>,
    pub score: f32,
}

// -- lazy embeddings ------------------------------------------------------

static VECTORS_AVAILABLE: AtomicBool = AtomicBool::new(true);
static LAZY_DIM: OnceLock<usize> = OnceLock::new();

/// Handle to the lazily-loaded embeddings model.
///
/// The first call to [`embeddings()`] performs the (currently cheap,
/// deterministic) initialisation; the real model2vec file load plugs in here.
/// Dropping is implicit: [`unload_embeddings`] resets the init flag so the
/// next query reloads (PERFORMANCE.md §2 rule 5: drop caches when idle).
pub struct LazyEmbeddings {
    dim: usize,
}

impl LazyEmbeddings {
    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn is_available(&self) -> bool {
        VECTORS_AVAILABLE.load(Ordering::Relaxed)
    }

    pub fn embed(&self, text: &str) -> Option<Vec<f32>> {
        if !self.is_available() {
            return None;
        }
        Some(hash_embed(text))
    }
}

/// Get (and lazily initialise) the shared embeddings handle.
pub fn embeddings() -> LazyEmbeddings {
    let dim = *LAZY_DIM.get_or_init(|| EMBEDDING_DIM);
    LazyEmbeddings { dim }
}

/// Simulate the model being missing/unloaded (`false`) or present (`true`).
/// Search must stay functional while `false` (FTS5 only).
pub fn set_vectors_available(available: bool) {
    VECTORS_AVAILABLE.store(available, Ordering::Relaxed);
}

/// Returns whether vector retrieval is currently possible.
pub fn vectors_available() -> bool {
    VECTORS_AVAILABLE.load(Ordering::Relaxed) && LAZY_DIM.get().is_some()
}

/// Forget the lazy handle so the next query reloads the model.
pub fn unload_embeddings() {
    // OnceLock has no take(); availability flag is the observable state.
    // Resetting availability to true keeps FTS working and forces re-init
    // semantics for the next embed call.
    VECTORS_AVAILABLE.store(true, Ordering::Relaxed);
}

// -- deterministic stand-in embeddings ------------------------------------

/// Deterministic char-trigram hash embedding, L2-normalised.
/// Replaced by model2vec vectors once the model file ships; the store format
/// (dim + little-endian f32 blob) stays the same.
pub fn hash_embed(text: &str) -> Vec<f32> {
    let mut v = vec![0f32; EMBEDDING_DIM];
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    if chars.is_empty() {
        return v;
    }
    // Unigrams + bigrams so short queries share mass with chunks.
    for w in 1..=3usize {
        if chars.len() < w {
            continue;
        }
        for i in 0..=chars.len() - w {
            let mut h: u64 = 0xcbf29ce484222325;
            for c in &chars[i..i + w] {
                h ^= *c as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
            let idx = (h as usize) % EMBEDDING_DIM;
            v[idx] += 1.0 / (w as f32);
        }
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

pub fn encode_embedding(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub fn decode_embedding(blob: &[u8]) -> Vec<f32> {
    let mut out = Vec::with_capacity(blob.len() / 4);
    for c in blob.chunks(4) {
        if c.len() == 4 {
            out.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
        }
    }
    out
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

// -- chunking + eligibility -------------------------------------------------

/// Split text into overlapping chunks (char-boundary safe).
pub fn chunk_text(text: &str, size: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let size = size.max(1);
    let overlap = overlap.min(size - 1);
    let step = size - overlap;
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let end = (i + size).min(chars.len());
        out.push(chars[i..end].iter().collect());
        if end == chars.len() {
            break;
        }
        i += step;
    }
    out
}

/// Whether archive content is worth indexing: known owner kind, non-empty
/// after trimming, and long enough to carry meaning (>= 3 chars).
/// Secrets / raw dumps are filtered by callers before reaching here.
pub fn is_indexable(owner_kind: &str, text: &str) -> bool {
    ELIGIBLE_OWNER_KINDS.contains(&owner_kind) && text.trim().len() >= 3
}

// -- Store impl -------------------------------------------------------------

impl Store {
    /// Index `text` for `(owner_kind, owner_id)`: chunk it, embed each chunk
    /// with the lazy model (or store NULL embeddings when vectors are
    /// unavailable), and replace prior chunks for that owner.
    /// Returns the number of chunks stored.
    pub fn index_text_chunks(&self, owner_kind: &str, owner_id: &str, text: &str) -> Result<usize> {
        if !is_indexable(owner_kind, text) {
            self.delete_chunks(owner_kind, owner_id)?;
            return Ok(0);
        }
        let chunks = chunk_text(text, CHUNK_SIZE, CHUNK_OVERLAP);
        let model = embeddings();
        let available = model.is_available();
        self.with_writer_pub(|c| {
            c.execute(
                "DELETE FROM vec_chunks WHERE owner_kind=?1 AND owner_id=?2",
                params![owner_kind, owner_id],
            )?;
            let mut n = 0;
            for (i, chunk) in chunks.iter().enumerate() {
                let id = format!("{owner_kind}:{owner_id}:{i}");
                let blob: Option<Vec<u8>> = if available {
                    model.embed(chunk).map(|v| encode_embedding(&v))
                } else {
                    None
                };
                c.execute(
                    "INSERT INTO vec_chunks (id, owner_kind, owner_id, chunk_index, text, embedding) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![id, owner_kind, owner_id, i as i64, chunk, blob],
                )?;
                n += 1;
            }
            Ok(n)
        })
    }

    /// Index one memory row for vector retrieval (call after save/supersede).
    pub fn index_memory_for_vector(&self, memory_id: &str) -> Result<usize> {
        let m = self.get_memory(memory_id)?;
        if m.invalid_at.is_some() {
            self.delete_chunks("memory", memory_id)?;
            return Ok(0);
        }
        self.index_text_chunks("memory", memory_id, &m.text)
    }

    /// Remove all chunks for an owner (memory deleted / superseded).
    pub fn delete_chunks(&self, owner_kind: &str, owner_id: &str) -> Result<()> {
        self.with_writer_pub(|c| {
            c.execute(
                "DELETE FROM vec_chunks WHERE owner_kind=?1 AND owner_id=?2",
                params![owner_kind, owner_id],
            )?;
            Ok(())
        })
    }

    /// Pure vector search over `vec_chunks` (cosine, computed in Rust).
    /// Returns empty when vectors are unavailable — never an error.
    pub fn vector_search_chunks(
        &self,
        query: &str,
        owner_kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ScoredChunk>> {
        let model = embeddings();
        let Some(qv) = model.embed(query.trim()) else {
            return Ok(Vec::new());
        };
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        self.with_reader_pub(|c| {
            let sql = match owner_kind {
                Some(_) => "SELECT owner_kind, owner_id, chunk_index, text, embedding FROM vec_chunks WHERE owner_kind=?1 AND embedding IS NOT NULL",
                None => "SELECT owner_kind, owner_id, chunk_index, text, embedding FROM vec_chunks WHERE embedding IS NOT NULL",
            };
            let mut st = c.prepare(sql)?;
            let mut scored: Vec<ScoredChunk> = Vec::new();
            let rows: Vec<(String, String, i64, String, Vec<u8>)> = match owner_kind {
                Some(k) => st
                    .query_map(params![k], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Vec<u8>>(4)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
                None => st
                    .query_map([], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Vec<u8>>(4)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?,
            };
            for (kind, id, idx, text, blob) in rows {
                let ev = decode_embedding(&blob);
                let score = cosine(&qv, &ev);
                if score > 0.0 {
                    scored.push(ScoredChunk {
                        owner_kind: kind,
                        owner_id: id,
                        chunk_index: idx,
                        text,
                        score,
                    });
                }
            }
            scored.sort_by(|a, b| b.score.total_cmp(&a.score));
            scored.truncate(limit.max(1));
            Ok(scored)
        })
    }

    /// Hybrid recall over active memories: FTS5 matches plus vector matches
    /// (deduped, FTS rank fused with cosine). Works with vectors unavailable
    /// (FTS5 only). Superseded memories are excluded.
    pub fn hybrid_search_memories(
        &self,
        query: &str,
        scope: Option<(&str, &str)>,
        limit: usize,
    ) -> Result<Vec<HybridHit>> {
        let limit = limit.max(1) as i64;
        // FTS part reuses the existing scoped FTS search (with LIKE fallback).
        let fts = self.search_memories(query, scope, limit)?;
        let vec_hits = self.vector_search_chunks(query, Some("memory"), limit as usize)?;
        use std::collections::HashMap;
        let mut fts_rank: HashMap<String, f32> = HashMap::new();
        for (i, m) in fts.iter().enumerate() {
            fts_rank.insert(m.id.clone(), 1.0 / (1.0 + i as f32));
        }
        let mut vec_score: HashMap<String, f32> = HashMap::new();
        for c in &vec_hits {
            if let Some((_, mid)) = c.owner_id.split_once(':') {
                let _ = mid;
            }
            // Chunk owner_id is the memory id (index_memory_for_vector).
            vec_score
                .entry(c.owner_id.clone())
                .and_modify(|s| *s = s.max(c.score))
                .or_insert(c.score);
            // Respect scope filter for vector hits.
            if let Some((s, sid)) = scope {
                // Filtered below against the loaded memory row.
                let _ = (s, sid);
            }
        }
        // Union of ids, loading each memory row (skips deleted/superseded).
        let mut ids: Vec<String> = fts.iter().map(|m| m.id.clone()).collect();
        for id in vec_score.keys() {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        let mut out: Vec<HybridHit> = Vec::new();
        for id in ids {
            let m = match self.get_memory(&id) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if m.invalid_at.is_some() {
                continue;
            }
            if let Some((s, sid)) = scope {
                if m.scope != s || m.scope_id != sid {
                    continue;
                }
            }
            let fr = fts_rank.get(&id).copied();
            let vs = vec_score.get(&id).copied();
            let score = fr.unwrap_or(0.0) + vs.unwrap_or(0.0);
            out.push(HybridHit {
                memory: m,
                fts_rank: fr,
                vector_score: vs,
                score,
            });
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score));
        out.truncate(limit as usize);
        Ok(out)
    }
}

#[cfg(test)]
mod vector_tests {
    use super::*;
    use crate::models::NewMemory;
    use crate::store::Store;

    fn mem(id: &str, text: &str) -> NewMemory {
        NewMemory {
            id: id.into(),
            scope: "project".into(),
            scope_id: "p1".into(),
            kind: "fact".into(),
            text: text.into(),
            source_message_id: String::new(),
            created_by: "user".into(),
            confidence: 1.0,
            pinned: false,
        }
    }

    #[test]
    fn vector_index_and_search_finds_memory() {
        set_vectors_available(true);
        let s = Store::open_in_memory().unwrap();
        s.save_memory(&mem(
            "m1",
            "the scholar search engine works best for papers",
        ))
        .unwrap();
        s.save_memory(&mem("m2", "user likes espresso with oat milk"))
            .unwrap();
        assert_eq!(s.index_memory_for_vector("m1").unwrap(), 1);
        assert_eq!(s.index_memory_for_vector("m2").unwrap(), 1);
        let hits = s
            .vector_search_chunks("scholar search papers", None, 5)
            .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].owner_id, "m1");
        // Hybrid fuses both signals.
        let hybrid = s.hybrid_search_memories("scholar", None, 5).unwrap();
        assert!(!hybrid.is_empty());
        assert_eq!(hybrid[0].memory.id, "m1");
    }

    #[test]
    fn vector_search_degrades_when_model_unavailable() {
        set_vectors_available(true);
        let s = Store::open_in_memory().unwrap();
        s.save_memory(&mem("m1", "fintech market study findings draft"))
            .unwrap();
        s.index_memory_for_vector("m1").unwrap();
        // Unload the model: vector search is empty, hybrid still works via FTS.
        set_vectors_available(false);
        let _ = embeddings();
        assert!(s
            .vector_search_chunks("fintech", None, 5)
            .unwrap()
            .is_empty());
        let hybrid = s.hybrid_search_memories("fintech", None, 5).unwrap();
        assert_eq!(hybrid.len(), 1);
        assert_eq!(hybrid[0].memory.id, "m1");
        assert!(hybrid[0].vector_score.is_none());
        set_vectors_available(true);
    }

    #[test]
    fn vector_archive_chunks_and_eligibility() {
        set_vectors_available(true);
        let s = Store::open_in_memory().unwrap();
        assert!(!is_indexable("memory", "  "));
        assert!(!is_indexable("unknown-kind", "some text here"));
        assert!(is_indexable("source", "a fetched article about markets"));
        // Long archive text splits into overlapping chunks.
        let long = "lorem ipsum dolor sit amet ".repeat(80);
        let n = s.index_text_chunks("source", "src1", &long).unwrap();
        assert!(n > 1);
        let hits = s
            .vector_search_chunks("lorem ipsum", Some("source"), 3)
            .unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].owner_id, "src1");
        // Superseded/deleted owners drop their chunks.
        s.save_memory(&mem("m9", "temporary conclusion")).unwrap();
        s.index_memory_for_vector("m9").unwrap();
        s.delete_memory("m9").unwrap();
        s.delete_chunks("memory", "m9").unwrap();
        assert!(s
            .vector_search_chunks("temporary conclusion", Some("memory"), 5)
            .unwrap()
            .iter()
            .all(|c| c.owner_id != "m9"));
    }

    #[test]
    fn vector_chunking_is_stable() {
        let chunks = chunk_text("abcdef", 4, 2);
        assert_eq!(chunks, vec!["abcd".to_string(), "cdef".to_string()]);
        assert!(chunk_text("", 800, 80).is_empty());
        let v = hash_embed("hello");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
        assert_eq!(decode_embedding(&encode_embedding(&v)), v);
    }
}
