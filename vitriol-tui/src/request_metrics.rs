//! Per-task measurements from the existing server timing log.

use std::collections::{BTreeMap, VecDeque};

use crate::model::SlotSnapshot;

#[derive(Clone, Debug, Default)]
pub struct CompletedRequest {
    pub slot: u64,
    pub task: u64,
    pub prompt_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub prompt_ms: Option<f64>,
    pub decoded_tokens: Option<u64>,
    pub decode_ms: Option<f64>,
    pub draft_accepted: Option<u64>,
    pub draft_proposed: Option<u64>,
}

/// At most one pending and one completed measurement per slot.
#[derive(Default)]
pub struct RequestMetrics {
    pending: BTreeMap<u64, CompletedRequest>,
    completed: VecDeque<CompletedRequest>,
}

impl RequestMetrics {
    pub fn completed(&self) -> Vec<CompletedRequest> {
        self.completed.iter().cloned().collect()
    }

    pub fn observe_slots(&mut self, slots: &[SlotSnapshot]) {
        for slot in slots.iter().filter(|slot| slot.is_processing) {
            let Some(task) = slot.id_task else { continue };
            if let Some(request) = self.pending.get_mut(&slot.id).filter(|r| r.task == task) {
                request.cached_tokens = slot.prompt_cached.or(request.cached_tokens);
            }
        }
    }

    pub fn record(&mut self, line: &str) {
        if line.starts_with("VITRIOL-FINGERPRINT:") || line.contains("loading model '") {
            *self = Self::default();
            return;
        }
        let Some(slot) = crate::logs::slot_id(line) else {
            return;
        };
        let Some((_, body)) = line.split_once(" | task ") else {
            return;
        };
        let Some((task, body)) = body.split_once(" | ") else {
            return;
        };
        let Ok(task) = task.trim().parse::<u64>() else {
            return;
        };
        if slot >= 64 {
            return;
        }
        if body.starts_with("processing task,") {
            self.pending.insert(
                slot,
                CompletedRequest {
                    slot,
                    task,
                    ..Default::default()
                },
            );
            return;
        }
        let Some(request) = self.pending.get_mut(&slot).filter(|r| r.task == task) else {
            return;
        };
        let body = body.trim_start();
        if let Some(timing) = body.strip_prefix("prompt eval time =") {
            if let Some((ms, tokens)) = parse_timing(timing) {
                request.prompt_ms = Some(ms);
                request.prompt_tokens = Some(tokens);
            }
        } else if let Some(timing) = body.strip_prefix("eval time =") {
            if let Some((ms, tokens)) = parse_timing(timing) {
                request.decode_ms = Some(ms);
                request.decoded_tokens = Some(tokens);
            }
        } else if let Some((_, counts)) = body.split_once(" accepted /") {
            let accepted = body
                .split_once('(')
                .and_then(|(_, n)| n.split_whitespace().next()?.parse().ok());
            let proposed = counts
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok());
            if accepted.zip(proposed).is_some_and(|(a, p)| a <= p) {
                request.draft_accepted = accepted;
                request.draft_proposed = proposed;
            }
        } else if body.starts_with("stop processing:") {
            let request = self.pending.remove(&slot).unwrap();
            if request.prompt_ms.is_some() && request.decode_ms.is_some() {
                self.completed.retain(|previous| previous.slot != slot);
                self.completed.push_front(request);
            }
        }
    }
}

fn parse_timing(text: &str) -> Option<(f64, u64)> {
    let (ms, rest) = text.split_once(" ms /")?;
    let ms = ms
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|ms| ms.is_finite() && *ms >= 0.0)?;
    let tokens = rest.split_whitespace().next()?.parse().ok()?;
    Some((ms, tokens))
}

#[derive(Clone, Debug)]
pub struct SlotRestore {
    pub slot: u64,
    pub tokens: u64,
    pub milliseconds: f64,
}

/// Sidecar restore events are accepted only when its following baseline PID
/// identifies the currently served engine.
pub fn restores_for_pid(lines: &[String], pid: Option<u32>) -> Vec<SlotRestore> {
    let Some(pid) = pid else { return Vec::new() };
    let mut pending = Vec::new();
    let mut matched = Vec::new();
    for line in lines {
        if line.contains("server healthy at ") || line.contains("server generation change:") {
            pending.clear();
            matched.clear();
        }
        if let Some((_, tail)) = line.split_once("restored slot") {
            if let Some((slot, tail)) = tail.split_once(".bin: ") {
                if let Some((tokens, timing)) = tail.split_once(" tokens (") {
                    let milliseconds = timing
                        .strip_suffix(" ms)")
                        .and_then(|n| n.parse::<f64>().ok())
                        .filter(|ms| ms.is_finite() && *ms >= 0.0);
                    if let (Ok(slot), Ok(tokens), Some(milliseconds)) =
                        (slot.parse(), tokens.parse(), milliseconds)
                    {
                        pending.push(SlotRestore {
                            slot,
                            tokens,
                            milliseconds,
                        });
                    }
                }
            }
        }
        if let Some((_, value)) = line.split_once("baseline server pid ") {
            matched = if value.trim().parse::<u32>().ok() == Some(pid) {
                std::mem::take(&mut pending)
            } else {
                pending.clear();
                Vec::new()
            };
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(slot: u64, task: u64, body: &str) -> String {
        format!("0.12.345 I slot print_timing: id {slot} | task {task} | {body}")
    }

    fn timing(metrics: &mut RequestMetrics, slot: u64, task: u64) {
        metrics.record(&line(
            slot,
            task,
            "prompt eval time = 1200.50 ms / 32 tokens (10 tokens per second)",
        ));
        metrics.record(&line(
            slot,
            task,
            "eval time = 250.00 ms / 8 tokens (32 tokens per second)",
        ));
    }

    #[test]
    fn publish_only_complete_matching_task_and_keep_sampled_cache() {
        let mut metrics = RequestMetrics::default();
        metrics.record(&line(0, 12, "processing task, is_child = 0"));
        metrics.observe_slots(&[SlotSnapshot {
            id: 0,
            id_task: Some(12),
            is_processing: true,
            prompt_cached: Some(200000),
            ..Default::default()
        }]);
        timing(&mut metrics, 0, 12);
        metrics.record(&line(
            0,
            12,
            "draft acceptance = 0.75 ( 6 accepted / 8 generated), mean len = 3",
        ));
        assert!(metrics.completed().is_empty());
        metrics.record(&line(0, 13, "stop processing: n_tokens = 1"));
        assert!(metrics.completed().is_empty());
        metrics.record(&line(0, 12, "stop processing: n_tokens = 200040"));
        let request = metrics.completed().pop().unwrap();
        assert_eq!(request.prompt_ms, Some(1200.5));
        assert_eq!(request.decode_ms, Some(250.0));
        assert_eq!(request.prompt_tokens, Some(32));
        assert_eq!(request.decoded_tokens, Some(8));
        assert_eq!(request.cached_tokens, Some(200000));
        assert_eq!(request.draft_accepted, Some(6));
        assert_eq!(request.draft_proposed, Some(8));
    }

    #[test]
    fn interleaved_slots_and_short_requests_do_not_inherit_cache() {
        let mut metrics = RequestMetrics::default();
        for slot in [0, 1] {
            metrics.record(&line(slot, 4, "processing task, is_child = 0"));
        }
        timing(&mut metrics, 1, 4);
        timing(&mut metrics, 0, 4);
        for slot in [1, 0] {
            metrics.record(&line(slot, 4, "stop processing: n_tokens = 40"));
        }
        assert_eq!(metrics.completed().len(), 2);
        assert_eq!(metrics.completed()[0].slot, 0);
        assert!(metrics
            .completed()
            .iter()
            .all(|r| r.cached_tokens.is_none()));
        metrics.record(&line(0, 5, "processing task, is_child = 0"));
        metrics.record(&line(0, 5, "prompt eval time = NaN ms / 32 tokens"));
        metrics.record(&line(0, 5, "stop processing: n_tokens = 1"));
        assert_eq!(metrics.completed()[0].task, 4);
        metrics.record("VITRIOL-FINGERPRINT: gpu=1");
        assert!(metrics.completed().is_empty());
    }

    #[test]
    fn cache_from_another_task_or_idle_slot_is_ignored() {
        let mut metrics = RequestMetrics::default();
        metrics.record(&line(0, 12, "processing task, is_child = 0"));
        for (task, active) in [(13, true), (12, false)] {
            metrics.observe_slots(&[SlotSnapshot {
                id: 0,
                id_task: Some(task),
                is_processing: active,
                prompt_cached: Some(999),
                ..Default::default()
            }]);
        }
        timing(&mut metrics, 0, 12);
        metrics.record(&line(0, 12, "stop processing: n_tokens = 40"));
        assert_eq!(metrics.completed()[0].cached_tokens, None);
    }

    #[test]
    fn missing_start_or_missing_timing_does_not_publish() {
        let mut metrics = RequestMetrics::default();
        timing(&mut metrics, 0, 1);
        metrics.record(&line(0, 1, "stop processing: n_tokens = 40"));
        assert!(metrics.completed().is_empty());
        metrics.record(&line(0, 2, "processing task, is_child = 0"));
        metrics.record(&line(0, 2, "eval time = -5 ms / 1 tokens"));
        metrics.record(&line(0, 2, "stop processing: n_tokens = 40"));
        assert!(metrics.completed().is_empty());
    }

    #[test]
    fn restore_requires_matching_current_pid_and_finite_duration() {
        let logs = vec![
            "server healthy at http://127.0.0.1:8279".into(),
            "restored slot0.bin: 203976 tokens (1966.998 ms)".into(),
            "baseline server pid 81".into(),
        ];
        assert!(restores_for_pid(&logs, None).is_empty());
        assert!(restores_for_pid(&logs, Some(82)).is_empty());
        let restore = restores_for_pid(&logs, Some(81));
        assert_eq!(restore[0].slot, 0);
        assert_eq!(restore[0].tokens, 203976);
        assert_eq!(restore[0].milliseconds, 1966.998);
        let mut restarted = logs;
        restarted.push("server generation change: 81 -> 82".into());
        assert!(restores_for_pid(&restarted, Some(81)).is_empty());
    }
}
