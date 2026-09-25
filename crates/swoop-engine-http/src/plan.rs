//! The segment table: which byte ranges exist, who is working on them, how far each has been
//! written and flushed. Every split, retry and checkpoint goes through here so the invariants
//! live in one place:
//!
//! * `committed <= written <= end` for every segment;
//! * `committed` only moves in [`Plan::commit`] after the caller's flush succeeded;
//! * segments are never removed (finished ones stay `Done`), so a `usize` handle is stable.

use std::path::PathBuf;
use std::time::Instant;
use swoop_domain::{Segment, SegmentMap, TaskError};
use tokio_util::sync::CancellationToken;

/// `end` of a segment whose size is unknown (chunked responses).
pub const UNKNOWN_END: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegStatus {
    /// Waiting for a worker (possibly after `not_before`).
    Pending,
    Running,
    Done,
}

#[derive(Debug)]
pub struct SegState {
    pub index: u32,
    pub start: u64,
    /// Exclusive. Shrinks when the segment is split.
    pub end: u64,
    /// First byte not yet handed to the writer. Workers restart from here after a retry
    /// because the writer never drops queued data (a sticky writer error fails the task).
    pub written: u64,
    /// First byte not yet on stable storage (checkpoint watermark).
    pub committed: u64,
    pub status: SegStatus,
    /// Mirror that served (or will serve) this segment.
    pub mirror: usize,
    /// Failed attempts so far (drives the backoff policy). Reset when an attempt makes
    /// progress: a long download must survive many *spaced-out* disconnects.
    pub attempts: u32,
    /// `written` when the current attempt started (progress detection).
    pub attempt_from: u64,
    /// Earliest time a pending segment may be started again.
    pub not_before: Option<Instant>,
    /// Bytes received since the controller last sampled (per-segment speed).
    pub window_bytes: u64,
    /// Token the controller cancels to shrink concurrency; `Some` only while running.
    pub stop: Option<CancellationToken>,
    pub last_error: Option<TaskError>,
}

impl SegState {
    fn new(index: u32, start: u64, end: u64, committed: u64) -> Self {
        Self {
            index,
            start,
            end,
            written: committed,
            committed,
            status: if committed >= end {
                SegStatus::Done
            } else {
                SegStatus::Pending
            },
            mirror: 0,
            attempts: 0,
            attempt_from: committed,
            not_before: None,
            window_bytes: 0,
            stop: None,
            last_error: None,
        }
    }

    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.written)
    }

    pub fn is_ready(&self, now: Instant) -> bool {
        self.status == SegStatus::Pending && self.not_before.is_none_or(|t| t <= now)
    }
}

#[derive(Debug)]
pub struct Plan {
    pub segments: Vec<SegState>,
    next_index: u32,
    /// `None` when the size is unknown (single segment to EOF).
    pub total: Option<u64>,
    pub min_segment: u64,
}

impl Plan {
    /// Fresh plan over a known size with `connections` equal parts (fewer if segments would
    /// fall under `min_segment`; one for tiny files).
    pub fn initial(total: u64, connections: usize, min_segment: u64) -> Self {
        let min = min_segment.max(1);
        let n = initial_segment_count(total, connections, min);
        let mut segments = Vec::with_capacity(n);
        let size = total.div_ceil(n as u64).max(1);
        let mut start = 0u64;
        let mut idx = 0u32;
        while start < total {
            let end = (start + size).min(total);
            segments.push(SegState::new(idx, start, end, start));
            start = end;
            idx += 1;
        }
        if segments.is_empty() {
            // zero-length file: one trivially complete segment so completion logic is uniform
            segments.push(SegState::new(0, 0, 0, 0));
            idx = 1;
        }
        Self {
            segments,
            next_index: idx,
            total: Some(total),
            min_segment: min,
        }
    }

    /// One segment from offset 0 to `total` (or to EOF when unknown).
    pub fn single(total: Option<u64>, min_segment: u64) -> Self {
        let end = total.unwrap_or(UNKNOWN_END);
        Self {
            segments: vec![SegState::new(0, 0, end, 0)],
            next_index: 1,
            total,
            min_segment: min_segment.max(1),
        }
    }

    /// Rebuild from a persisted map: everything from `committed` onward is fetched again.
    pub fn from_map(map: &SegmentMap, min_segment: u64) -> Self {
        let mut segments: Vec<SegState> = map
            .segments
            .iter()
            .map(|s| {
                let committed = s.committed.clamp(s.start, s.end);
                let mut st = SegState::new(s.index, s.start, s.end, committed);
                st.mirror = s.source_index as usize;
                st
            })
            .collect();
        segments.sort_by_key(|s| s.start);
        let next_index = segments.iter().map(|s| s.index + 1).max().unwrap_or(0);
        Self {
            segments,
            next_index,
            total: map.total,
            min_segment: min_segment.max(1),
        }
    }

    pub fn running(&self) -> usize {
        self.segments
            .iter()
            .filter(|s| s.status == SegStatus::Running)
            .count()
    }

    pub fn pending(&self) -> usize {
        self.segments
            .iter()
            .filter(|s| s.status == SegStatus::Pending)
            .count()
    }

    pub fn is_complete(&self) -> bool {
        self.segments.iter().all(|s| s.status == SegStatus::Done)
    }

    /// Bytes handed to the writer so far (progress).
    pub fn written_bytes(&self) -> u64 {
        self.segments
            .iter()
            .map(|s| s.written.saturating_sub(s.start))
            .sum()
    }

    pub fn committed_bytes(&self) -> u64 {
        self.segments
            .iter()
            .map(|s| s.committed.saturating_sub(s.start))
            .sum()
    }

    /// Largest ready pending segment.
    pub fn pick_ready(&self, now: Instant) -> Option<usize> {
        self.segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_ready(now))
            .max_by_key(|(_, s)| s.remaining())
            .map(|(i, _)| i)
    }

    /// Earliest `not_before` among pending segments that are not ready yet.
    pub fn next_wake(&self) -> Option<Instant> {
        self.segments
            .iter()
            .filter(|s| s.status == SegStatus::Pending)
            .filter_map(|s| s.not_before)
            .min()
    }

    /// Whether any running segment has enough left to be split in two halves ≥ `min_segment`.
    pub fn splittable_running(&self) -> Option<usize> {
        self.total?;
        self.segments
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.status == SegStatus::Running && s.remaining() >= 2 * self.min_segment
            })
            .max_by_key(|(_, s)| s.remaining())
            .map(|(i, _)| i)
    }

    /// Split segment `i` at the midpoint of its remaining range. The running worker keeps the
    /// first half (it notices the new `end` on its next chunk); the second half becomes a new
    /// pending segment. Returns the new segment's handle.
    pub fn split(&mut self, i: usize) -> Option<usize> {
        self.total?;
        let (new_start, end, mirror) = {
            let s = self.segments.get(i)?;
            if s.status == SegStatus::Done || s.remaining() < 2 * self.min_segment {
                return None;
            }
            (s.written + s.remaining() / 2, s.end, s.mirror)
        };
        let index = self.next_index;
        self.next_index += 1;
        self.segments[i].end = new_start;
        let mut n = SegState::new(index, new_start, end, new_start);
        n.mirror = mirror;
        self.segments.push(n);
        Some(self.segments.len() - 1)
    }

    pub fn mark_running(&mut self, i: usize, mirror: usize, token: CancellationToken) {
        if let Some(s) = self.segments.get_mut(i) {
            s.status = SegStatus::Running;
            s.mirror = mirror;
            s.stop = Some(token);
            s.not_before = None;
            s.attempt_from = s.written;
        }
    }

    pub fn mark_done(&mut self, i: usize) {
        if let Some(s) = self.segments.get_mut(i) {
            s.status = SegStatus::Done;
            s.stop = None;
            if s.end == UNKNOWN_END {
                s.end = s.written;
            }
            s.written = s.written.max(s.end);
        }
    }

    /// Return a segment to the pending pool (retry, shrink, mirror switch).
    pub fn mark_pending(&mut self, i: usize, not_before: Option<Instant>) {
        if let Some(s) = self.segments.get_mut(i) {
            if s.written >= s.end && s.end != UNKNOWN_END {
                s.status = SegStatus::Done;
            } else {
                s.status = SegStatus::Pending;
            }
            s.stop = None;
            s.not_before = not_before;
        }
    }

    /// Non-resumable mode: a retry must start from byte 0 again.
    pub fn reset_to_start(&mut self, i: usize) -> u64 {
        match self.segments.get_mut(i) {
            Some(s) => {
                let discarded = s.written.saturating_sub(s.start);
                s.written = s.start;
                s.committed = s.start;
                discarded
            }
            None => 0,
        }
    }

    /// Advance `written` for a running segment; returns the number of bytes actually
    /// accepted (less than `len` if the segment was shrunk by a split meanwhile).
    pub fn advance(&mut self, i: usize, offset: u64, len: u64) -> u64 {
        let Some(s) = self.segments.get_mut(i) else {
            return 0;
        };
        if offset != s.written {
            return 0;
        }
        let accept = if s.end == UNKNOWN_END {
            len
        } else {
            len.min(s.end.saturating_sub(s.written))
        };
        s.written += accept;
        s.window_bytes += accept;
        accept
    }

    /// Snapshot of `written` per segment, taken *before* a flush.
    pub fn snapshot_written(&self) -> Vec<u64> {
        self.segments.iter().map(|s| s.written).collect()
    }

    /// Advance `committed` to the pre-flush snapshot (never beyond `end`).
    pub fn commit(&mut self, snapshot: &[u64]) {
        for (s, w) in self.segments.iter_mut().zip(snapshot.iter()) {
            let bound = if s.end == UNKNOWN_END { *w } else { s.end };
            s.committed = s.committed.max((*w).min(bound));
        }
    }

    /// Per-segment bytes since the last sample, resetting the windows.
    pub fn take_windows(&mut self) -> Vec<(usize, u64)> {
        self.segments
            .iter_mut()
            .enumerate()
            .filter(|(_, s)| s.status == SegStatus::Running)
            .map(|(i, s)| (i, std::mem::take(&mut s.window_bytes)))
            .collect()
    }

    pub fn to_map(
        &self,
        etag: Option<String>,
        last_modified: Option<String>,
        part_path: Option<PathBuf>,
    ) -> SegmentMap {
        SegmentMap {
            segments: self
                .segments
                .iter()
                .map(|s| Segment {
                    index: s.index,
                    start: s.start,
                    end: if s.end == UNKNOWN_END {
                        s.written
                    } else {
                        s.end
                    },
                    committed: s.committed,
                    source_index: s.mirror as u32,
                })
                .collect(),
            etag,
            last_modified,
            total: self.total,
            part_path,
        }
    }
}

/// How many segments to open initially: `connections`, bounded so every segment is at least
/// `min_segment`; files under twice the minimum are not worth splitting at all.
pub fn initial_segment_count(total: u64, connections: usize, min_segment: u64) -> usize {
    let min = min_segment.max(1);
    if total < 2 * min {
        return 1;
    }
    let by_size = (total / min).max(1) as usize;
    connections
        .clamp(1, crate::hostlimits::ABSOLUTE_MAX_CONNECTIONS)
        .min(by_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_segmentation_respects_minimum() {
        assert_eq!(initial_segment_count(1000, 8, 1024), 1);
        assert_eq!(initial_segment_count(3 * 1024, 8, 1024), 3);
        assert_eq!(initial_segment_count(100 << 20, 8, 1 << 20), 8);
        assert_eq!(initial_segment_count(100 << 20, 200, 1 << 20), 64);
        let p = Plan::initial(10, 3, 1);
        let ranges: Vec<(u64, u64)> = p.segments.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(ranges, vec![(0, 4), (4, 8), (8, 10)]);
        let z = Plan::initial(0, 4, 1);
        assert!(z.is_complete());
    }

    #[test]
    fn split_and_commit() {
        let mut p = Plan::initial(100, 1, 10);
        let tok = CancellationToken::new();
        p.mark_running(0, 0, tok);
        assert_eq!(p.advance(0, 0, 20), 20);
        let n = p.split(0).unwrap();
        assert_eq!(p.segments[0].end, 60);
        assert_eq!((p.segments[n].start, p.segments[n].end), (60, 100));
        assert_eq!(p.segments[n].index, 1);
        // a chunk overlapping the new end is truncated
        assert_eq!(p.advance(0, 20, 50), 40);
        assert_eq!(p.segments[0].written, 60);
        let snap = p.snapshot_written();
        p.commit(&snap);
        assert_eq!(p.committed_bytes(), 60);
        p.mark_done(0);
        assert!(!p.is_complete());
        assert_eq!(p.pick_ready(Instant::now()), Some(n));
        // 40 bytes left: splittable with a 10-byte minimum, not with a 25-byte one
        p.min_segment = 25;
        assert!(p.split(n).is_none());
        let map = p.to_map(None, None, None);
        assert_eq!(map.committed_bytes(), 60);
        let back = Plan::from_map(&map, 10);
        assert_eq!(back.segments[0].status, SegStatus::Done);
        assert_eq!(back.segments[1].written, 60);
    }

    #[test]
    fn unknown_size_single() {
        let mut p = Plan::single(None, 1);
        p.mark_running(0, 0, CancellationToken::new());
        assert_eq!(p.advance(0, 0, 7), 7);
        assert!(p.splittable_running().is_none());
        p.mark_done(0);
        assert_eq!(p.segments[0].end, 7);
        assert!(p.is_complete());
        assert_eq!(p.to_map(None, None, None).segments[0].end, 7);
    }
}
