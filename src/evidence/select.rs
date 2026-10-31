//! Deterministic log selection for evidence bundles.
//!
//! Logs are always returned in chronological order and capped at
//! [`MAX_LOGS`]. When the cap is exceeded, failure and WARN/ERROR rows are
//! prioritized, the first and last INFO rows frame the window, immediate
//! neighbors of priority rows are kept, and the remaining slots are filled
//! chronologically. Selection precedence when the cap is tight is:
//! priority rows, first/last INFO, neighbors, then chronological fill.
//!
//! If more than [`MAX_LOGS`] priority rows exist, the earliest [`MAX_LOGS`]
//! priority rows are kept and no INFO context is retained; the omission is
//! reported through [`LogSelection::omitted`] / [`LogSelection::truncated`].

use std::collections::BTreeSet;

use crate::domain::LogRecord;

/// Maximum number of logs kept in an evidence bundle.
pub const MAX_LOGS: usize = 200;

/// Selected logs plus the metadata required by the evidence contract.
#[derive(Debug, Clone, PartialEq)]
pub struct LogSelection {
    /// Selected logs in chronological order.
    pub items: Vec<LogRecord>,
    /// Number of logs supplied before selection.
    pub total: usize,
    /// Number of logs left out (`total - items.len()`).
    pub omitted: usize,
    /// True when at least one log was left out.
    pub truncated: bool,
}

/// Selects the most relevant logs for an evidence bundle.
pub fn select_logs(logs: Vec<LogRecord>) -> LogSelection {
    let total = logs.len();

    if total <= MAX_LOGS {
        let mut items = logs;
        sort_chronologically(&mut items);
        return LogSelection {
            items,
            total,
            omitted: 0,
            truncated: false,
        };
    }

    let mut ordered = logs;
    sort_chronologically(&mut ordered);

    let priority: Vec<usize> = ordered
        .iter()
        .enumerate()
        .filter(|(_, log)| is_priority(&log.severity))
        .map(|(index, _)| index)
        .collect();

    let selected_indices = if priority.len() > MAX_LOGS {
        // The cap is fully consumed by failure rows: keep the earliest ones
        // deterministically and report the rest as omitted.
        priority.into_iter().take(MAX_LOGS).collect()
    } else {
        select_with_context(&ordered, &priority)
    };

    let items: Vec<LogRecord> = selected_indices
        .into_iter()
        .map(|index| ordered[index].clone())
        .collect();
    let omitted = total - items.len();

    LogSelection {
        items,
        total,
        omitted,
        truncated: omitted > 0,
    }
}

/// Picks up to [`MAX_LOGS`] indices from chronologically ordered `logs`.
fn select_with_context(logs: &[LogRecord], priority: &[usize]) -> BTreeSet<usize> {
    let total = logs.len();
    let mut selected = BTreeSet::new();

    // Priority rows always come first and already fit within the cap.
    selected.extend(priority.iter().copied());

    // The first and last INFO rows frame the retained window.
    let first_info = logs.iter().position(|log| is_info(&log.severity));
    let last_info = logs.iter().rposition(|log| is_info(&log.severity));
    for index in [first_info, last_info].into_iter().flatten() {
        if selected.len() >= MAX_LOGS {
            break;
        }
        selected.insert(index);
    }

    // Keep the logs immediately before and after each priority row.
    'neighbors: for &index in priority {
        for neighbor in [index.checked_sub(1), Some(index + 1)]
            .into_iter()
            .flatten()
        {
            if selected.len() >= MAX_LOGS {
                break 'neighbors;
            }
            if neighbor < total {
                selected.insert(neighbor);
            }
        }
    }

    // Fill the remaining slots deterministically in chronological order.
    for index in 0..total {
        if selected.len() >= MAX_LOGS {
            break;
        }
        selected.insert(index);
    }

    selected
}

/// Sorts by timestamp; equal timestamps use content as a stable tie-break so
/// the result does not depend on the order rows arrived in.
fn sort_chronologically(logs: &mut [LogRecord]) {
    logs.sort_by(|left, right| {
        left.ts
            .cmp(&right.ts)
            .then_with(|| left.message.cmp(&right.message))
            .then_with(|| left.severity.cmp(&right.severity))
            .then_with(|| left.span_id.cmp(&right.span_id))
    });
}

fn is_priority(severity: &str) -> bool {
    let severity = severity.trim().to_ascii_uppercase();
    matches!(
        severity.as_str(),
        "WARN" | "WARNING" | "ERROR" | "FATAL" | "CRITICAL"
    ) || severity.contains("FAIL")
}

fn is_info(severity: &str) -> bool {
    let severity = severity.trim();
    severity.eq_ignore_ascii_case("info") || severity.eq_ignore_ascii_case("information")
}
