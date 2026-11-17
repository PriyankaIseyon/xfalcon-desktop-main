//! Markdown section for the diagnostics report, built from the local audit log.
//!
//! The section holds only the fields listed here: the integrity result, counts,
//! time range, pruning and repair flags, the dropped-event count, and the most
//! recent events as identifiers, actions and outcomes. It never includes event
//! content, the MAC key, or chain hashes.

use chrono::{DateTime, SecondsFormat, Utc};

use super::{AuditHub, LocalAuditStore};
use crate::domain::error::AppError;

const SECTION_TITLE: &str = "## 4. Security Audit Trail";

#[must_use]
pub fn render_diagnostics_section(
    store: &LocalAuditStore,
    hub: Option<&AuditHub>,
    max_events: usize,
) -> String {
    let report = match store.verify() {
        Ok(report) => report,
        Err(error) => return unreadable(&error),
    };
    let records = match store.recent(max_events) {
        Ok(records) => records,
        Err(error) => return unreadable(&error),
    };

    let mut out = format!("{SECTION_TITLE}\n\n");
    if report.ok {
        out.push_str("Integrity: verified\n\n");
    } else {
        let problem = report.problem.as_deref().unwrap_or("unknown problem");
        out.push_str(&format!("Integrity: FAILED — {problem}\n\n"));
    }

    let time_range = match (report.first_ts_ms, report.last_ts_ms) {
        (Some(first), Some(last)) => format!("{} to {}", rfc3339(first), rfc3339(last)),
        _ => "none".to_owned(),
    };
    let dropped = hub.map_or_else(
        || "not tracked".to_owned(),
        |hub| hub.dropped_events().to_string(),
    );
    out.push_str(&format!(
        "- Records: {}\n- Segments: {}\n- Time range: {}\n- Older segments pruned: {}\n- Torn tail repaired: {}\n- Previous log damaged and set aside: {}\n- Dropped events this session: {}\n\n",
        report.records,
        report.segments,
        time_range,
        yes_no(report.anchored_after_prune),
        yes_no(report.torn_tail_repaired),
        yes_no(store.recovered_from_damage()),
        dropped,
    ));

    if records.is_empty() {
        out.push_str("_No audit events recorded._\n");
        return out;
    }
    out.push_str("```text\n");
    for record in &records {
        let event = record.event();
        let detail = event
            .detail()
            .map_or_else(String::new, |detail| format!(" detail={detail}"));
        out.push_str(&format!(
            "{} seq={} {} {}:{} outcome={} actor={}{}\n",
            rfc3339(event.occurred_at_ms()),
            record.seq(),
            event.action().as_str(),
            event.resource_type().as_str(),
            event.resource_id(),
            event.outcome().as_str(),
            event.actor_id(),
            detail,
        ));
    }
    out.push_str("```\n");
    out
}

fn unreadable(error: &AppError) -> String {
    format!(
        "{SECTION_TITLE}\n\n_The audit trail could not be read._ Error kind: {}\n",
        error.code()
    )
}

fn rfc3339(ts_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ts_ms).map_or_else(
        || "invalid-timestamp".to_owned(),
        |instant| instant.to_rfc3339_opts(SecondsFormat::Millis, true),
    )
}

const fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}
