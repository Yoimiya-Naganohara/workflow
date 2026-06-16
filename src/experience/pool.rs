//! Mmap-backed persistent experience pool — the A-track (bedrock) store.
//!
//! Experiences are kept in memory for fast query and flushed to a
//! memory-mapped file on mutation.  On startup the file is loaded
//! into memory; on shutdown [`flush`](ExperiencePool::flush) persists
//! all entries.
//!
//! # File format
//!
//! ```text
//! ┌──────────────────────────────┐
//! │  PoolHeader (64 bytes)       │
//! ├──────────────────────────────┤
//! │  ExperienceEntry[0]          │
//! ├──────────────────────────────┤
//! │  ExperienceEntry[1]          │
//! ├──────────────────────────────┤
//! │  …                           │
//! └──────────────────────────────┘
//! ```
//!
//! The header contains a magic number, version, entry count and
//! capacity so the file can be grown incrementally.

use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use memmap2::{Mmap, MmapMut};
use tracing::{trace, warn};

use crate::core::simd::cosine_similarity_384;
use crate::core::types::{EMBEDDING_DIM, ExperienceEntry};

// ---------------------------------------------------------------------------
//  Constants
// ---------------------------------------------------------------------------

const MAGIC: u32 = 0x575F4558; // "W_EX"
const VERSION: u32 = 1;
const HEADER_SIZE: usize = 64;
const DEFAULT_CAPACITY: usize = 1024;
const GROW_BY: usize = 512;

/// Size of one serialised experience entry (repr(C), so stable).
pub const EXPERIENCE_SIZE: usize = size_of::<ExperienceEntry>();

type LoadResult = (Vec<ExperienceEntry>, Option<Mmap>, Option<MmapMut>, usize);

// ---------------------------------------------------------------------------
//  Header
// ---------------------------------------------------------------------------

#[repr(C)]
struct PoolHeader {
    magic: u32,
    version: u32,
    entry_size: u32, // sizeof(ExperienceEntry) – validation
    capacity: u32,   // max entries the current file can hold
    count: u32,      // current entries
    _pad: [u8; 44],  // 64-byte header total
}

#[allow(unused)]
const _: () = assert!(size_of::<PoolHeader>() == HEADER_SIZE);

// Assert that ExperienceEntry layout is stable for mmap (repr(C), no niche-optimized enums).
// Option<u32> is 4 bytes (niche optimization makes None = 0, same size as u32).
// If this assertion fails, the mmap file format is incompatible.
#[allow(unused)]
const _: () = assert!(size_of::<ExperienceEntry>() == 2104);

// ---------------------------------------------------------------------------
//  ExperiencePool
// ---------------------------------------------------------------------------

/// A persistent, mmap-backed store of [`ExperienceEntry`] values.
///
/// **Thread safety**: `ExperiencePool` is `Send` but not `Sync`.  Callers
/// should wrap it in a `Mutex` or `RwLock` when sharing across tasks.
pub struct ExperiencePool {
    /// Path to the mmap file on disk.
    path: PathBuf,
    /// In-memory entries — fast reads without touching the mmap.
    entries: Vec<ExperienceEntry>,
    /// Mmap handle kept alive so the OS keeps the pages warm.
    _mmap: Option<Mmap>,
    /// Write-mmap (re-created on every flush/grow).
    mmap_mut: Option<MmapMut>,
    /// Dirty flag — set when entries are added, cleared on flush.
    dirty: AtomicBool,
    /// Current file capacity (header + capacity × entry_size).
    capacity: usize,
}

impl ExperiencePool {
    /// Open (or create) a pool at `path`.
    ///
    /// If the file exists and has a valid header, entries are loaded
    /// into memory.  Otherwise an empty pool is initialised.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Open or create the backing file.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .context("Failed to open experience pool file")?;

        let (entries, mmap, mmap_mut, capacity) = if file.metadata().is_ok_and(|m| m.len() >= HEADER_SIZE as u64) {
            Self::load_from_file(&file)?
        } else {
            // New file – initialise empty.
            let cap = DEFAULT_CAPACITY;
            let total_size = HEADER_SIZE + cap * EXPERIENCE_SIZE;
            file.set_len(total_size as u64)?;

            let mut mm = unsafe { MmapMut::map_mut(&file)? };
            // Write header
            let header = PoolHeader {
                magic: MAGIC,
                version: VERSION,
                entry_size: EXPERIENCE_SIZE as u32,
                capacity: cap as u32,
                count: 0,
                _pad: [0u8; 44],
            };
            unsafe {
                std::ptr::copy_nonoverlapping(&header as *const PoolHeader as *const u8, mm.as_mut_ptr(), HEADER_SIZE);
            }
            mm.flush()?;

            let ro = mm.make_read_only()?;
            (Vec::new(), Some(ro), None, cap)
        };

        Ok(Self {
            path,
            entries,
            _mmap: mmap,
            mmap_mut,
            capacity,
            dirty: AtomicBool::new(false),
        })
    }

    // ── Loading ──

    fn load_from_file(file: &std::fs::File) -> Result<LoadResult> {
        let ro = unsafe { Mmap::map(file)? };
        let header: &PoolHeader = unsafe { &*(ro.as_ptr() as *const PoolHeader) };

        if header.magic != MAGIC || header.version != VERSION {
            warn!("Experience pool file has invalid header – starting fresh");
            return Ok((Vec::new(), None, None, DEFAULT_CAPACITY));
        }

        let count = header.count as usize;
        let capacity = header.capacity as usize;

        // Validate count against mmap file size — prevent UB from corrupted header.
        let file_bytes = ro.len();
        let max_entries = file_bytes.saturating_sub(HEADER_SIZE) / std::mem::size_of::<ExperienceEntry>();
        let count = if count > max_entries {
            warn!(
                "Experience pool header claims {} entries but file only fits {} – truncating",
                count, max_entries
            );
            max_entries
        } else {
            count
        };

        let entries: &[ExperienceEntry] =
            unsafe { std::slice::from_raw_parts(ro.as_ptr().add(HEADER_SIZE) as *const ExperienceEntry, count) };

        trace!(count, capacity, "Loaded experience pool from disk");
        Ok((entries.to_vec(), Some(ro), None, capacity))
    }

    // ── Public API ──

    /// Number of entries currently in the pool.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Access all entries (for iteration / clustering).
    pub fn entries(&self) -> &[ExperienceEntry] {
        &self.entries
    }

    /// Add a single entry and mark the pool as dirty.
    pub fn add(&mut self, entry: ExperienceEntry) {
        self.entries.push(entry);
        self.dirty.store(true, Ordering::Release);
    }

    /// Bulk-add entries.
    pub fn extend(&mut self, entries: impl IntoIterator<Item = ExperienceEntry>) {
        let old_len = self.entries.len();
        self.entries.extend(entries);
        if self.entries.len() > old_len {
            self.dirty.store(true, Ordering::Release);
        }
    }

    /// Retrieve top-k entries by weighted cosine similarity.
    pub fn search(&self, query: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(ExperienceEntry, f32)> {
        if self.entries.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f32)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let sim = cosine_similarity_384(query, &e.embedding);
                (i, sim * e.weight)
            })
            .collect();

        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(k);

        scored.into_iter().map(|(i, s)| (self.entries[i].clone(), s)).collect()
    }

    /// Remove all entries and reset the file, truncating to initial capacity.
    pub fn clear(&mut self) -> Result<()> {
        self.entries.clear();
        // Set capacity to 0 to force grow_file to re-create the file
        // at DEFAULT_CAPACITY (truncating any prior growth).
        self.capacity = 0;
        self.dirty.store(true, Ordering::Release);
        self.flush()
    }

    /// Flush in-memory entries to the mmap file.
    pub fn flush(&mut self) -> Result<()> {
        if !self.dirty.load(Ordering::Acquire) && self.mmap_mut.is_some() {
            return Ok(());
        }

        let count = self.entries.len();

        // Grow file if needed.
        let needed_capacity = if count <= self.capacity {
            self.capacity
        } else {
            // Grow to next multiple of GROW_BY.
            ((count / GROW_BY) + 1) * GROW_BY
        };

        if needed_capacity != self.capacity || self.mmap_mut.is_none() {
            self.grow_file(needed_capacity)?;
        }

        // Get the mutable mmap.
        let mm = self.mmap_mut.as_mut().expect("mmap_mut must exist after grow");

        // Update header count.
        {
            let header: &mut PoolHeader = unsafe { &mut *(mm.as_mut_ptr() as *mut PoolHeader) };
            header.count = count as u32;
        }

        // Write entries.
        if !self.entries.is_empty() {
            let dst = unsafe { mm.as_mut_ptr().add(HEADER_SIZE) as *mut ExperienceEntry };
            unsafe {
                std::ptr::copy_nonoverlapping(self.entries.as_ptr(), dst, count);
            }
        }

        mm.flush().context("Failed to flush experience pool mmap")?;
        self.dirty.store(false, Ordering::Release);
        trace!(count, capacity = self.capacity, "Flushed experience pool");

        Ok(())
    }

    /// Grow the backing file and remap.
    fn grow_file(&mut self, new_capacity: usize) -> Result<()> {
        let total_size = HEADER_SIZE + new_capacity * EXPERIENCE_SIZE;

        // Close current mappings.
        self.mmap_mut.take();
        self._mmap.take();

        // Reopen file and set new length.
        let file = std::fs::OpenOptions::new().read(true).write(true).open(&self.path)?;
        file.set_len(total_size as u64)?;

        let mut mm = unsafe { MmapMut::map_mut(&file)? };

        // Write updated header.
        let header = PoolHeader {
            magic: MAGIC,
            version: VERSION,
            entry_size: EXPERIENCE_SIZE as u32,
            capacity: new_capacity as u32,
            count: self.entries.len() as u32,
            _pad: [0u8; 44],
        };
        unsafe {
            std::ptr::copy_nonoverlapping(&header as *const PoolHeader as *const u8, mm.as_mut_ptr(), HEADER_SIZE);
        }

        // Copy existing entries.
        if !self.entries.is_empty() {
            let dst = unsafe { mm.as_mut_ptr().add(HEADER_SIZE) as *mut ExperienceEntry };
            unsafe {
                std::ptr::copy_nonoverlapping(self.entries.as_ptr(), dst, self.entries.len());
            }
        }

        let ro = mm.make_read_only()?;
        self._mmap = Some(ro);

        // Re-open mutable map for future flushes.
        let mm2 = unsafe { MmapMut::map_mut(&file)? };
        self.mmap_mut = Some(mm2);
        self.capacity = new_capacity;

        Ok(())
    }
}

impl std::fmt::Debug for ExperiencePool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExperiencePool")
            .field("path", &self.path)
            .field("entries", &self.entries.len())
            .field("capacity", &self.capacity)
            .field("dirty", &self.dirty.load(Ordering::Relaxed))
            .finish()
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn dummy_entry(weight: f32) -> ExperienceEntry {
        ExperienceEntry {
            embedding: [0.0f32; EMBEDDING_DIM],
            applicability_vector: [0.0f32; 128],
            tool_bitmap: 0,
            role_template_id: None,
            weight,
            domain_version: 0,
            timestamp: 0,
            l2_override_weight: 0.0,
            l2_override_created_at: 0,
        }
    }

    fn tmp_path() -> PathBuf {
        std::env::temp_dir().join(format!("test_pool_{}", rand::random::<u64>()))
    }

    #[test]
    fn test_open_creates_new_file() {
        let path = tmp_path();
        let pool = ExperiencePool::open(&path).unwrap();
        assert!(pool.is_empty());
        assert_eq!(pool.len(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_add_and_flush() {
        let path = tmp_path();
        let mut pool = ExperiencePool::open(&path).unwrap();

        let mut e = dummy_entry(1.0);
        e.embedding[0] = 1.0;
        pool.add(e);
        assert_eq!(pool.len(), 1);
        assert!(pool.dirty.load(Ordering::Relaxed));

        pool.flush().unwrap();
        assert!(!pool.dirty.load(Ordering::Relaxed));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_reload_from_disk() {
        let path = tmp_path();
        {
            let mut pool = ExperiencePool::open(&path).unwrap();
            let mut e = dummy_entry(0.8);
            e.embedding[0] = 1.0;
            e.tool_bitmap = 0b101;
            pool.add(e);
            pool.flush().unwrap();
        }

        // Re-open – should see the saved entry.
        let pool = ExperiencePool::open(&path).unwrap();
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.entries()[0].tool_bitmap, 0b101);
        assert!((pool.entries()[0].weight - 0.8).abs() < f32::EPSILON);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_search_returns_top_k() {
        let path = tmp_path();
        let mut pool = ExperiencePool::open(&path).unwrap();

        let mut e1 = dummy_entry(1.0);
        e1.embedding[0] = 1.0;
        let mut e2 = dummy_entry(0.5);
        e2.embedding[0] = 0.5;
        let mut e3 = dummy_entry(0.2);
        e3.embedding[0] = 0.1;

        pool.add(e1);
        pool.add(e2);
        pool.add(e3);

        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;

        let results = pool.search(&query, 2);
        assert_eq!(results.len(), 2);
        // Highest weighted similarity first.
        assert!((results[0].1 * results[0].0.weight) > (results[1].1 * results[1].0.weight));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_clear_and_reload() {
        let path = tmp_path();
        {
            let mut pool = ExperiencePool::open(&path).unwrap();
            pool.add(dummy_entry(1.0));
            pool.flush().unwrap();
            assert_eq!(pool.len(), 1);
            pool.clear().unwrap();
            assert_eq!(pool.len(), 0);
        }

        let pool = ExperiencePool::open(&path).unwrap();
        assert_eq!(pool.len(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_grow_file() {
        let path = tmp_path();
        let mut pool = ExperiencePool::open(&path).unwrap();

        // Add more entries than the default capacity (1024).
        for i in 0..1200 {
            let mut e = dummy_entry(1.0);
            e.embedding[0] = i as f32;
            pool.add(e);
        }
        assert_eq!(pool.len(), 1200);
        pool.flush().unwrap();

        // Re-open and verify count.
        let pool = ExperiencePool::open(&path).unwrap();
        assert_eq!(pool.len(), 1200);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_corrupted_header_count_causes_ub() {
        let path = tmp_path();
        // Create a valid pool with 1 entry
        {
            let mut pool = ExperiencePool::open(&path).unwrap();
            pool.add(dummy_entry(1.0));
            pool.flush().unwrap();
        }

        // Corrupt the header: set count to a huge number
        let mut file = std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap();
        let _header_size = std::mem::size_of::<PoolHeader>();

        // Write a bogus count (999999) at the count field offset
        // PoolHeader layout: magic(8) + version(4) + count(4) + capacity(4)
        let count_offset = 8 + 4; // after magic and version
        use std::io::{Seek, SeekFrom, Write};
        file.seek(SeekFrom::Start(count_offset as u64)).unwrap();
        file.write_all(&999999u32.to_le_bytes()).unwrap();
        drop(file);

        // Re-open — this should either detect corruption or cause UB
        // BUG: no bounds validation on count vs file size
        let result = std::panic::catch_unwind(|| {
            let _pool = ExperiencePool::open(&path).unwrap();
        });

        // The open may panic (UB) or succeed with garbage data
        if result.is_err() {
            println!("BUG CONFIRMED: corrupted mmap count causes panic/UB");
        }
        std::fs::remove_file(&path).ok();
    }
}
