//! Mirror handling: several URLs serving identical bytes. Mirrors are probed in parallel,
//! those disagreeing on the size are dropped (ETags may legitimately differ between hosts),
//! the rest are ranked by probe latency and then by measured throughput. Segments are handed
//! out with a smooth weighted round-robin so a fast mirror gets proportionally more of them.

use crate::probe::{data_url, probe, ProbeResult};
use crate::request::RequestTemplate;
use std::time::Duration;
use swoop_domain::{ErrorKind, TaskError};
use swoop_runtime::redact::redact;

#[derive(Clone, Debug)]
pub struct Mirror {
    pub url: String,
    pub healthy: bool,
    pub latency: Duration,
    pub failures: u32,
    pub bytes: u64,
    /// Exponentially smoothed throughput in bytes/s (0 until measured).
    pub speed: f64,
    /// Segments assigned so far (weighted round-robin bookkeeping).
    picks: u64,
    pub last_error: Option<TaskError>,
}

impl Mirror {
    fn weight(&self) -> f64 {
        if self.speed > 1.0 {
            self.speed
        } else {
            // Before any throughput is known, rank by latency: 10 ms → 100, 1 s → ~1.
            1_000.0 / (self.latency.as_millis() as f64 + 10.0)
        }
    }
}

/// After this many failures a mirror is not tried again in this run.
pub const MAX_FAILURES_PER_MIRROR: u32 = 3;

#[derive(Debug)]
pub struct MirrorSet {
    mirrors: Vec<Mirror>,
}

impl MirrorSet {
    pub fn single(url: String) -> Self {
        Self {
            mirrors: vec![Mirror {
                url,
                healthy: true,
                latency: Duration::ZERO,
                failures: 0,
                bytes: 0,
                speed: 0.0,
                picks: 0,
                last_error: None,
            }],
        }
    }

    pub fn len(&self) -> usize {
        self.mirrors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mirrors.is_empty()
    }

    pub fn is_multi(&self) -> bool {
        self.mirrors.len() > 1
    }

    pub fn url(&self, i: usize) -> &str {
        self.mirrors
            .get(i)
            .map(|m| m.url.as_str())
            .unwrap_or_default()
    }

    pub fn healthy_count(&self) -> usize {
        self.mirrors.iter().filter(|m| m.healthy).count()
    }

    pub fn get(&self, i: usize) -> Option<&Mirror> {
        self.mirrors.get(i)
    }

    /// Choose the next mirror for a segment: healthy, not `exclude`, lowest `picks / weight`.
    pub fn pick(&mut self, exclude: Option<usize>) -> Option<usize> {
        let best = self
            .mirrors
            .iter()
            .enumerate()
            .filter(|(i, m)| m.healthy && Some(*i) != exclude)
            .min_by(|(_, a), (_, b)| {
                let ka = a.picks as f64 / a.weight();
                let kb = b.picks as f64 / b.weight();
                ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .or_else(|| {
                // Only the excluded mirror is left: better than giving up.
                exclude.filter(|&i| self.mirrors.get(i).map(|m| m.healthy).unwrap_or(false))
            })?;
        self.mirrors[best].picks += 1;
        Some(best)
    }

    /// Record a failure; returns `true` when the mirror just became unhealthy.
    pub fn record_failure(&mut self, i: usize, error: &TaskError, fatal: bool) -> bool {
        let Some(m) = self.mirrors.get_mut(i) else {
            return false;
        };
        m.failures += 1;
        m.last_error = Some(error.clone());
        let was = m.healthy;
        if fatal || m.failures >= MAX_FAILURES_PER_MIRROR {
            m.healthy = false;
        }
        was && !m.healthy
    }

    pub fn mark_unhealthy(&mut self, i: usize, error: Option<TaskError>) {
        if let Some(m) = self.mirrors.get_mut(i) {
            m.healthy = false;
            if error.is_some() {
                m.last_error = error;
            }
        }
    }

    /// Feed a throughput sample (bytes over `dt`) into the mirror's speed estimate.
    pub fn record_throughput(&mut self, i: usize, bytes: u64, dt: Duration) {
        let Some(m) = self.mirrors.get_mut(i) else {
            return;
        };
        m.bytes += bytes;
        let secs = dt.as_secs_f64();
        if secs <= 0.0 || bytes == 0 {
            return;
        }
        let sample = bytes as f64 / secs;
        m.speed = if m.speed <= 0.0 {
            sample
        } else {
            0.7 * m.speed + 0.3 * sample
        };
    }

    /// The error to report when no mirror is left.
    pub fn exhausted_error(&self) -> TaskError {
        let detail: Vec<String> = self
            .mirrors
            .iter()
            .map(|m| {
                format!(
                    "{}: {}",
                    redact(&m.url),
                    m.last_error
                        .as_ref()
                        .map(|e| e.message.clone())
                        .unwrap_or_else(|| "ok".into())
                )
            })
            .collect();
        TaskError::new(
            ErrorKind::MirrorExhausted,
            format!("all {} mirrors failed", self.mirrors.len()),
        )
        .with_detail(detail.join("; "))
        .with_source(redact(self.url(0)))
    }
}

/// Probe every mirror in parallel. The first mirror (in list order) that answers defines the
/// reference size; mirrors with a different size are dropped, failed ones are kept but marked
/// unhealthy (they may recover later in the run through explicit re-probing — not done
/// today). Returns the set and the reference probe.
pub async fn probe_all(
    client: &reqwest::Client,
    template: &RequestTemplate,
    urls: &[String],
    timeout: Duration,
) -> Result<(MirrorSet, ProbeResult, Vec<(usize, TaskError)>), TaskError> {
    let futures: Vec<_> = urls
        .iter()
        .map(|u| async move { probe(client, template, u, timeout).await })
        .collect();
    let results = futures::future::join_all(futures).await;

    let reference = results
        .iter()
        .enumerate()
        .find_map(|(i, r)| r.as_ref().ok().map(|p| (i, p.clone())));
    let Some((_, reference)) = reference else {
        let first_err = results
            .into_iter()
            .find_map(|r| r.err())
            .unwrap_or_else(|| TaskError::new(ErrorKind::MirrorExhausted, "no mirrors"));
        return Err(first_err);
    };

    let mut mirrors = Vec::new();
    let mut rejected = Vec::new();
    for (i, (url, r)) in urls.iter().zip(results).enumerate() {
        match r {
            Ok(p) => {
                if p.total() != reference.total() {
                    rejected.push((
                        i,
                        TaskError::new(
                            ErrorKind::SourceChanged,
                            format!(
                                "mirror size {:?} differs from primary {:?}",
                                p.total(),
                                reference.total()
                            ),
                        )
                        .with_source(redact(url)),
                    ));
                    continue;
                }
                mirrors.push(Mirror {
                    url: data_url(url, &p.final_url),
                    healthy: true,
                    latency: p.latency,
                    failures: 0,
                    bytes: 0,
                    speed: 0.0,
                    picks: 0,
                    last_error: None,
                });
            }
            Err(e) => {
                rejected.push((i, e.clone()));
                mirrors.push(Mirror {
                    url: url.clone(),
                    healthy: false,
                    latency: timeout,
                    failures: 1,
                    bytes: 0,
                    speed: 0.0,
                    picks: 0,
                    last_error: Some(e),
                });
            }
        }
    }
    Ok((MirrorSet { mirrors }, reference, rejected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_round_robin_prefers_fast_mirrors() {
        let mut set = MirrorSet {
            mirrors: (0..2)
                .map(|i| Mirror {
                    url: format!("http://m{i}/f"),
                    healthy: true,
                    latency: Duration::from_millis(10),
                    failures: 0,
                    bytes: 0,
                    speed: if i == 0 { 3_000_000.0 } else { 1_000_000.0 },
                    picks: 0,
                    last_error: None,
                })
                .collect(),
        };
        let mut counts = [0usize; 2];
        for _ in 0..40 {
            counts[set.pick(None).unwrap()] += 1;
        }
        assert!(counts[0] > counts[1] * 2, "{counts:?}");
        // failures drop a mirror; exclusion falls back when it is the only one left
        let e = TaskError::new(ErrorKind::ServerError, "x");
        assert!(!set.record_failure(1, &e, false));
        assert!(!set.record_failure(1, &e, false));
        assert!(set.record_failure(1, &e, false));
        assert_eq!(set.healthy_count(), 1);
        assert_eq!(set.pick(Some(0)), Some(0));
        set.mark_unhealthy(0, None);
        assert_eq!(set.pick(None), None);
        assert_eq!(set.exhausted_error().kind, ErrorKind::MirrorExhausted);
    }
}
