//! CSR graph mmap cache (format v1).
//!
//! The file `.brain/graph.mmap` is compiled from SQLite by [`CsrCompiler`] and
//! read by [`CsrMmapGraph`]. It enables fast neighborhood expansion for agent
//! context without re-querying SQLite for every hop.
//!
//! Full layout: repository `docs/MMAP_FORMAT.md`.
//!
//! # Design notes
//!
//! - All multi-byte integers are **little-endian**.
//! - Sections are **size-validated** before any access.
//! - Neighbor/vector accessors use explicit `from_le_bytes` loads (no unaligned
//!   `from_raw_parts` casts) — correct on all platforms and free of UB under
//!   mmap base-alignment variance.
//! - Publish path is **write temp + `rename`** for crash safety.

#[cfg(feature = "mmap")]
mod imp {
    use crate::error::{BrainError, Result};
    use memmap2::Mmap;
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    /// On-disk format version for `graph.mmap`.
    pub const MMAP_VERSION: u32 = 1;
    /// Header size in bytes.
    pub const HEADER_SIZE: usize = 64;
    /// Magic bytes: `RBRNMAP1`
    pub const MAGIC_BYTES: &[u8; 8] = b"RBRNMAP1";

    /// Flag bit: id string table is present after the vector matrix.
    pub const FLAG_HAS_IDS: u32 = 1;

    /// Compiler that bakes nodes, edges, and optional vectors into `graph.mmap`.
    pub struct CsrCompiler;

    impl CsrCompiler {
        /// Compile CSR graph and atomically replace `output_path` (write temp + rename).
        pub fn compile<P: AsRef<Path>>(
            output_path: P,
            node_ids: &[String],
            edges: &[(String, String, f32)],
            vectors: Option<&[f32]>,
            vector_dim: usize,
        ) -> Result<()> {
            let output_path = output_path.as_ref();
            let bytes = Self::encode(node_ids, edges, vectors, vector_dim)?;

            let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
            std::fs::create_dir_all(parent)?;

            let tmp_path = unique_tmp_path(output_path);
            {
                let mut file = File::create(&tmp_path)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
            std::fs::rename(&tmp_path, output_path)?;
            Ok(())
        }

        /// Encode the binary layout into a buffer (useful for tests).
        pub fn encode(
            node_ids: &[String],
            edges: &[(String, String, f32)],
            vectors: Option<&[f32]>,
            vector_dim: usize,
        ) -> Result<Vec<u8>> {
            let node_count = node_ids.len();
            if let Some(vecs) = vectors {
                if vector_dim == 0 || vecs.len() != node_count * vector_dim {
                    return Err(BrainError::mmap(
                        "vector buffer length must equal node_count * vector_dim",
                    ));
                }
            } else if vector_dim != 0 {
                return Err(BrainError::mmap(
                    "vector_dim must be 0 when vectors are absent",
                ));
            }

            let mut id_to_index: HashMap<&str, u32> = HashMap::with_capacity(node_count);
            for (idx, id) in node_ids.iter().enumerate() {
                id_to_index.insert(id.as_str(), idx as u32);
            }

            let mut adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); node_count];
            for (src, dst, weight) in edges {
                if let (Some(&src_idx), Some(&dst_idx)) =
                    (id_to_index.get(src.as_str()), id_to_index.get(dst.as_str()))
                {
                    adj[src_idx as usize].push((dst_idx, *weight));
                }
            }

            let mut row_offsets: Vec<u32> = Vec::with_capacity(node_count + 1);
            let mut targets: Vec<u32> = Vec::new();
            let mut weights: Vec<f32> = Vec::new();
            let mut current_offset = 0u32;
            row_offsets.push(current_offset);
            for neighbors in &adj {
                for &(target_idx, w) in neighbors {
                    targets.push(target_idx);
                    weights.push(w);
                    current_offset = current_offset.checked_add(1).ok_or_else(|| {
                        BrainError::mmap("edge count overflow")
                    })?;
                }
                row_offsets.push(current_offset);
            }
            let edge_count = targets.len();

            // Estimate capacity.
            let mut buf: Vec<u8> = Vec::with_capacity(
                HEADER_SIZE
                    + (node_count + 1) * 4
                    + edge_count * 8
                    + node_count * vector_dim * 4
                    + node_ids.iter().map(|s| 2 + s.len()).sum::<usize>(),
            );

            // Header
            let mut header = [0u8; HEADER_SIZE];
            header[0..8].copy_from_slice(MAGIC_BYTES);
            header[8..12].copy_from_slice(&MMAP_VERSION.to_le_bytes());
            header[12..16].copy_from_slice(&(node_count as u32).to_le_bytes());
            header[16..20].copy_from_slice(&(edge_count as u32).to_le_bytes());
            header[20..24].copy_from_slice(&(vector_dim as u32).to_le_bytes());
            header[24..28].copy_from_slice(&FLAG_HAS_IDS.to_le_bytes());
            buf.extend_from_slice(&header);

            for offset in &row_offsets {
                buf.extend_from_slice(&offset.to_le_bytes());
            }
            for target in &targets {
                buf.extend_from_slice(&target.to_le_bytes());
            }
            for w in &weights {
                buf.extend_from_slice(&w.to_le_bytes());
            }
            if let Some(vecs) = vectors {
                for v in vecs {
                    buf.extend_from_slice(&v.to_le_bytes());
                }
            }

            // ID string table (always written when FLAG_HAS_IDS is set).
            for id in node_ids {
                let bytes = id.as_bytes();
                if bytes.len() > u16::MAX as usize {
                    return Err(BrainError::mmap(format!(
                        "node id longer than u16::MAX: {}",
                        id
                    )));
                }
                buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
                buf.extend_from_slice(bytes);
            }

            Ok(buf)
        }
    }

    fn unique_tmp_path(output: &Path) -> PathBuf {
        let mut tmp = output.as_os_str().to_os_string();
        tmp.push(".tmp");
        // Include pid to reduce collision under concurrent writers.
        tmp.push(format!(".{}", std::process::id()));
        PathBuf::from(tmp)
    }

    /// Memory-mapped CSR graph reader with safe unaligned loads.
    #[derive(Debug)]
    pub struct CsrMmapGraph {
        _file: File,
        mmap: Mmap,
        /// Number of nodes in the CSR graph.
        pub node_count: usize,
        /// Number of edges.
        pub edge_count: usize,
        /// Embedding dimension (`0` when no vectors are stored).
        pub vector_dim: usize,
        /// Header flags bitfield.
        pub flags: u32,
        offsets_byte_start: usize,
        targets_byte_start: usize,
        weights_byte_start: usize,
        vectors_byte_start: usize,
        _ids_byte_start: usize,
        /// Parsed once at open for O(1) id→index and index→id.
        node_ids: Vec<String>,
        id_to_index: HashMap<String, u32>,
    }

    impl CsrMmapGraph {
        /// Open and fully validate a `graph.mmap` file.
        pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
            let file = File::open(path.as_ref())?;
            let mmap = unsafe { Mmap::map(&file) }.map_err(|e| {
                BrainError::mmap(format!("failed to memory-map graph file: {e}"))
            })?;
            Self::from_bytes(file, mmap)
        }

        fn from_bytes(file: File, mmap: Mmap) -> Result<Self> {
            if mmap.len() < HEADER_SIZE {
                return Err(BrainError::mmap("file too small for header"));
            }
            if &mmap[0..8] != MAGIC_BYTES {
                return Err(BrainError::mmap(format!(
                    "invalid magic bytes (found {:?})",
                    String::from_utf8_lossy(&mmap[0..8])
                )));
            }

            let version = read_u32(&mmap, 8)?;
            if version != MMAP_VERSION {
                return Err(BrainError::mmap(format!(
                    "unsupported mmap version {version} (expected {MMAP_VERSION})"
                )));
            }

            let node_count = read_u32(&mmap, 12)? as usize;
            let edge_count = read_u32(&mmap, 16)? as usize;
            let vector_dim = read_u32(&mmap, 20)? as usize;
            let flags = read_u32(&mmap, 24)?;

            let offsets_byte_start = HEADER_SIZE;
            let targets_byte_start = offsets_byte_start
                .checked_add(node_count.checked_add(1).ok_or_else(|| BrainError::mmap("overflow"))? * 4)
                .ok_or_else(|| BrainError::mmap("overflow"))?;
            let weights_byte_start = targets_byte_start
                .checked_add(edge_count * 4)
                .ok_or_else(|| BrainError::mmap("overflow"))?;
            let vectors_byte_start = weights_byte_start
                .checked_add(edge_count * 4)
                .ok_or_else(|| BrainError::mmap("overflow"))?;
            let ids_byte_start = vectors_byte_start
                .checked_add(node_count * vector_dim * 4)
                .ok_or_else(|| BrainError::mmap("overflow"))?;

            // Minimum size without IDs.
            if mmap.len() < ids_byte_start {
                return Err(BrainError::mmap(format!(
                    "truncated file: need at least {ids_byte_start} bytes, have {}",
                    mmap.len()
                )));
            }

            let mut node_ids = Vec::with_capacity(node_count);
            let mut id_to_index = HashMap::with_capacity(node_count);
            let mut cursor = ids_byte_start;

            if flags & FLAG_HAS_IDS != 0 {
                for i in 0..node_count {
                    if cursor + 2 > mmap.len() {
                        return Err(BrainError::mmap("truncated id table (length prefix)"));
                    }
                    let len = read_u16(&mmap, cursor)? as usize;
                    cursor += 2;
                    if cursor + len > mmap.len() {
                        return Err(BrainError::mmap("truncated id table (payload)"));
                    }
                    let s = std::str::from_utf8(&mmap[cursor..cursor + len])
                        .map_err(|e| BrainError::mmap(format!("invalid utf-8 in id table: {e}")))?
                        .to_string();
                    cursor += len;
                    id_to_index.insert(s.clone(), i as u32);
                    node_ids.push(s);
                }
            } else {
                // Synthesize synthetic ids if absent (legacy/test).
                for i in 0..node_count {
                    let s = format!("node-{i}");
                    id_to_index.insert(s.clone(), i as u32);
                    node_ids.push(s);
                }
            }

            // Validate CSR offsets monotonicity and bounds.
            if node_count > 0 {
                let mut prev = 0u32;
                for i in 0..=node_count {
                    let off = read_u32(&mmap, offsets_byte_start + i * 4)?;
                    if off < prev || off as usize > edge_count {
                        return Err(BrainError::mmap(format!(
                            "invalid row offset at {i}: {off} (prev={prev}, edge_count={edge_count})"
                        )));
                    }
                    prev = off;
                }
                // Final offset must equal edge_count.
                let last = read_u32(&mmap, offsets_byte_start + node_count * 4)?;
                if last as usize != edge_count {
                    return Err(BrainError::mmap(format!(
                        "final row offset {last} != edge_count {edge_count}"
                    )));
                }
            }

            // Validate target indices.
            for i in 0..edge_count {
                let t = read_u32(&mmap, targets_byte_start + i * 4)? as usize;
                if t >= node_count {
                    return Err(BrainError::mmap(format!(
                        "edge target {t} out of range (node_count={node_count})"
                    )));
                }
            }

            let _ = cursor; // remaining trailing bytes ignored (forward compatible)

            Ok(Self {
                _file: file,
                mmap,
                node_count,
                edge_count,
                vector_dim,
                flags,
                offsets_byte_start,
                targets_byte_start,
                weights_byte_start,
                vectors_byte_start,
                _ids_byte_start: ids_byte_start,
                node_ids,
                id_to_index,
            })
        }

        /// Look up the node id string for a CSR index.
        pub fn node_id(&self, idx: usize) -> Option<&str> {
            self.node_ids.get(idx).map(|s| s.as_str())
        }

        /// Look up the CSR index for a node id.
        pub fn index_of(&self, id: &str) -> Option<u32> {
            self.id_to_index.get(id).copied()
        }

        /// All node ids in CSR index order.
        pub fn node_ids(&self) -> &[String] {
            &self.node_ids
        }

        /// Graph neighbors as owned vectors (safe, allocation per call).
        pub fn get_neighbors(&self, node_idx: usize) -> (Vec<u32>, Vec<f32>) {
            if node_idx >= self.node_count {
                return (Vec::new(), Vec::new());
            }
            let start = read_u32(&self.mmap, self.offsets_byte_start + node_idx * 4)
                .unwrap_or(0) as usize;
            let end = read_u32(&self.mmap, self.offsets_byte_start + (node_idx + 1) * 4)
                .unwrap_or(0) as usize;
            if start > end || end > self.edge_count {
                return (Vec::new(), Vec::new());
            }
            let mut targets = Vec::with_capacity(end - start);
            let mut weights = Vec::with_capacity(end - start);
            for i in start..end {
                let t = read_u32(&self.mmap, self.targets_byte_start + i * 4).unwrap_or(0);
                let w = read_f32(&self.mmap, self.weights_byte_start + i * 4).unwrap_or(0.0);
                targets.push(t);
                weights.push(w);
            }
            (targets, weights)
        }

        /// k-hop neighborhood with multiplicative path weights (first visit wins).
        pub fn k_hop_neighborhood(&self, start_node_idx: usize, k: usize) -> Vec<(u32, f32)> {
            let mut visited: HashMap<u32, f32> = HashMap::new();
            let mut current_frontier = vec![(start_node_idx as u32, 1.0f32)];

            for _ in 0..k {
                let mut next_frontier = Vec::new();
                for (curr_node, curr_w) in current_frontier {
                    let (targets, weights) = self.get_neighbors(curr_node as usize);
                    for i in 0..targets.len() {
                        let target = targets[i];
                        if target as usize == start_node_idx {
                            continue;
                        }
                        let weight = weights[i] * curr_w;
                        if let std::collections::hash_map::Entry::Vacant(e) =
                            visited.entry(target)
                        {
                            e.insert(weight);
                            next_frontier.push((target, weight));
                        }
                    }
                }
                current_frontier = next_frontier;
            }

            visited.into_iter().collect()
        }

        /// Copy out the vector row for `node_idx` (None if no vectors).
        pub fn get_vector(&self, node_idx: usize) -> Option<Vec<f32>> {
            if self.vector_dim == 0 || node_idx >= self.node_count {
                return None;
            }
            let start = self.vectors_byte_start + node_idx * self.vector_dim * 4;
            let mut out = Vec::with_capacity(self.vector_dim);
            for i in 0..self.vector_dim {
                out.push(read_f32(&self.mmap, start + i * 4).ok()?);
            }
            Some(out)
        }

        /// Dot product with an unrolled loop (may auto-vectorize; not explicit SIMD).
        pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
            let len = a.len().min(b.len());
            let mut sum = 0.0f32;
            let chunks_a = a[..len].chunks_exact(8);
            let chunks_b = b[..len].chunks_exact(8);
            let remainder_a = chunks_a.remainder();
            let remainder_b = chunks_b.remainder();
            for (ca, cb) in chunks_a.zip(chunks_b) {
                sum += ca[0] * cb[0]
                    + ca[1] * cb[1]
                    + ca[2] * cb[2]
                    + ca[3] * cb[3]
                    + ca[4] * cb[4]
                    + ca[5] * cb[5]
                    + ca[6] * cb[6]
                    + ca[7] * cb[7];
            }
            for (ra, rb) in remainder_a.iter().zip(remainder_b.iter()) {
                sum += ra * rb;
            }
            sum
        }

        /// Full scan top-k by dot product (acceptable for small N in v0.1).
        pub fn top_k_vector_search(&self, query_vector: &[f32], top_k: usize) -> Vec<(u32, f32)> {
            if self.vector_dim == 0 || query_vector.len() != self.vector_dim {
                return Vec::new();
            }
            let mut scores: Vec<(u32, f32)> = Vec::with_capacity(self.node_count);
            for idx in 0..self.node_count {
                if let Some(vec_row) = self.get_vector(idx) {
                    let score = Self::dot_product(query_vector, &vec_row);
                    scores.push((idx as u32, score));
                }
            }
            scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            scores.truncate(top_k);
            scores
        }
    }

    fn read_u16(buf: &[u8], off: usize) -> Result<u16> {
        let end = off.checked_add(2).ok_or_else(|| BrainError::mmap("overflow"))?;
        if end > buf.len() {
            return Err(BrainError::mmap("read_u16 out of bounds"));
        }
        Ok(u16::from_le_bytes(buf[off..end].try_into().unwrap()))
    }

    fn read_u32(buf: &[u8], off: usize) -> Result<u32> {
        let end = off.checked_add(4).ok_or_else(|| BrainError::mmap("overflow"))?;
        if end > buf.len() {
            return Err(BrainError::mmap("read_u32 out of bounds"));
        }
        Ok(u32::from_le_bytes(buf[off..end].try_into().unwrap()))
    }

    fn read_f32(buf: &[u8], off: usize) -> Result<f32> {
        let end = off.checked_add(4).ok_or_else(|| BrainError::mmap("overflow"))?;
        if end > buf.len() {
            return Err(BrainError::mmap("read_f32 out of bounds"));
        }
        Ok(f32::from_le_bytes(buf[off..end].try_into().unwrap()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use tempfile::tempdir;

        #[test]
        fn compile_open_query() {
            let dir = tempdir().unwrap();
            let mmap_path = dir.path().join("graph.mmap");

            let node_ids = vec![
                "docs/a".to_string(),
                "docs/b".to_string(),
                "docs/c".to_string(),
            ];
            let edges = vec![
                ("docs/a".to_string(), "docs/b".to_string(), 0.9f32),
                ("docs/a".to_string(), "docs/c".to_string(), 0.5f32),
                ("docs/b".to_string(), "docs/c".to_string(), 0.8f32),
            ];
            let vectors = vec![
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                1.0, 1.0, 0.0, 0.0,
            ];

            CsrCompiler::compile(&mmap_path, &node_ids, &edges, Some(&vectors), 4).unwrap();
            let graph = CsrMmapGraph::open(&mmap_path).unwrap();
            assert_eq!(graph.node_count, 3);
            assert_eq!(graph.edge_count, 3);
            assert_eq!(graph.vector_dim, 4);
            assert_eq!(graph.node_id(0), Some("docs/a"));
            assert_eq!(graph.index_of("docs/b"), Some(1));

            let (targets, weights) = graph.get_neighbors(0);
            assert_eq!(targets, vec![1, 2]);
            assert!((weights[0] - 0.9).abs() < 1e-6);

            let neighbors = graph.k_hop_neighborhood(0, 1);
            assert_eq!(neighbors.len(), 2);

            let top = graph.top_k_vector_search(&[1.0, 0.0, 0.0, 0.0], 2);
            assert_eq!(top[0].0, 0);
            assert!((top[0].1 - 1.0).abs() < 1e-6);
        }

        #[test]
        fn rejects_truncated_file() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("bad.mmap");
            std::fs::write(&path, b"RBRNMAP1").unwrap();
            let err = CsrMmapGraph::open(&path).unwrap_err();
            assert!(matches!(err, BrainError::MmapFormat(_)));
        }

        #[test]
        fn rejects_bad_magic() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("bad.mmap");
            let mut buf = vec![0u8; 64];
            buf[0..8].copy_from_slice(b"NOTRIGHT");
            std::fs::write(&path, buf).unwrap();
            assert!(CsrMmapGraph::open(&path).is_err());
        }

        #[test]
        fn atomic_replace_leaves_valid_file() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("graph.mmap");
            let ids = vec!["n0".into()];
            CsrCompiler::compile(&path, &ids, &[], None, 0).unwrap();
            CsrCompiler::compile(&path, &ids, &[], None, 0).unwrap();
            let g = CsrMmapGraph::open(&path).unwrap();
            assert_eq!(g.node_count, 1);
        }
    }
}

#[cfg(feature = "mmap")]
pub use imp::*;

#[cfg(not(feature = "mmap"))]
mod stub {
    use crate::error::{BrainError, Result};
    use std::path::Path;

    pub const MMAP_VERSION: u32 = 1;
    pub const HEADER_SIZE: usize = 64;
    pub const MAGIC_BYTES: &[u8; 8] = b"RBRNMAP1";

    pub struct CsrCompiler;
    impl CsrCompiler {
        pub fn compile<P: AsRef<Path>>(
            _output_path: P,
            _node_ids: &[String],
            _edges: &[(String, String, f32)],
            _vectors: Option<&[f32]>,
            _vector_dim: usize,
        ) -> Result<()> {
            Err(BrainError::FeatureDisabled("mmap"))
        }
    }

    pub struct CsrMmapGraph;
    impl CsrMmapGraph {
        pub fn open<P: AsRef<Path>>(_path: P) -> Result<Self> {
            Err(BrainError::FeatureDisabled("mmap"))
        }
    }
}

#[cfg(not(feature = "mmap"))]
pub use stub::*;
