use serde_json::Value;

const MAX_TOKEN_COUNT: usize = 32;
const MAX_TOKEN_NAME_BYTES: usize = 32;
const MAX_SESSION_BYTES: usize = 64;
const MAX_TURN_BYTES: usize = 20;
const MAX_TIMESTAMP_BYTES: usize = 20;
const TERMINAL_MAX_AGE_MS: u64 = 30_000;
const TERMINAL_MAX_FUTURE_SKEW_MS: u64 = 5_000;
const OUTCOME_AUTHORITY: &str = "omp:v1";

/// The terminal state reported by the OMP companion extension.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum AgentOutcome {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl AgentOutcome {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// A validated, source-owned outcome metadata report. `session` is the
/// lowercase SHA-256 identity emitted by the companion extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OutcomeReport {
    pub(crate) session: String,
    pub(crate) turn: String,
    pub(crate) outcome: AgentOutcome,
    pub(crate) at_unix_ms: u64,
}

impl OutcomeReport {
    /// Parse the flat `AgentInfo.tokens` object without inspecting unrelated
    /// metadata values. Herdr bounds the map at 32 entries and token names at
    /// 32 ASCII bytes; repeat those limits here before reading our fields.
    pub(crate) fn parse_tokens(tokens: &Value) -> Option<Self> {
        let object = tokens.as_object()?;
        if object.len() > MAX_TOKEN_COUNT {
            return None;
        }
        if object.keys().any(|name| !valid_token_name(name)) {
            return None;
        }

        let authority = owned_string(object, "pet_outcome_authority", OUTCOME_AUTHORITY.len())?;
        if authority != OUTCOME_AUTHORITY {
            return None;
        }

        let session = owned_string(object, "pet_outcome_session", MAX_SESSION_BYTES)?;
        if !valid_session_token(session) {
            return None;
        }
        let turn = owned_string(object, "pet_outcome_turn", MAX_TURN_BYTES)?;
        let outcome_text = owned_string(object, "pet_outcome", 9)?;
        let at_text = owned_string(object, "pet_outcome_at", MAX_TIMESTAMP_BYTES)?;
        let outcome = AgentOutcome::parse(outcome_text)?;
        if outcome.is_terminal() && at_text.is_empty() {
            return None;
        }
        // Parse and canonicalize-check the owned numeric strings. Keeping the
        // original bounded strings in the report lets the caller compare the
        // opaque wire identity while the tracker compares its numeric order.
        parse_decimal(turn, false)?;
        let at_unix_ms = parse_decimal(at_text, true)?;

        Some(Self {
            session: session.to_owned(),
            turn: turn.to_owned(),
            outcome,
            at_unix_ms,
        })
    }
}

/// Reject malformed keys even when they belong to another metadata producer.
fn valid_token_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_TOKEN_NAME_BYTES
        && bytes.iter().all(|byte| {
            byte.is_ascii_uppercase()
                || byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || *byte == b'_'
                || *byte == b'-'
        })
}

fn valid_session_token(value: &str) -> bool {
    value.len() == MAX_SESSION_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Read one of our string-valued tokens. Values belonging to other producers
/// are intentionally never visited, so a large unrelated value cannot make a
/// valid outcome report fail.
fn owned_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    name: &str,
    max_bytes: usize,
) -> Option<&'a str> {
    let value = object.get(name)?.as_str()?;
    if value.is_empty() || value.len() > max_bytes || !value.is_ascii() {
        return None;
    }
    Some(value)
}

/// Parse a canonical unsigned decimal token. Turn zero is not a valid turn
/// identity; timestamp zero is retained as syntactically valid so freshness
/// remains the policy decision in `OutcomeTracker`.
fn parse_decimal(value: &str, allow_zero: bool) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || (bytes.len() > 1 && bytes[0] == b'0') {
        return None;
    }
    if !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let number = value.parse::<u64>().ok()?;
    if !allow_zero && number == 0 {
        return None;
    }
    Some(number)
}

/// Stateful acceptance gate for one Herdr source/session generation.
///
/// Construct a fresh tracker after a source generation or OMP session change.
/// Compare `OutcomeReport.session` with the current hashed agent session before
/// calling `observe`. A prior coherent snapshot distinguishes new, short-lived
/// turns from cached terminal metadata without requiring a running snapshot.
#[derive(Debug)]
pub(crate) struct OutcomeTracker {
    session: Option<String>,
    latest_running_turn: Option<u64>,
    last_terminal_turn: Option<u64>,
    baseline_unix_ms: Option<u64>,
}

impl OutcomeTracker {
    pub(crate) fn new(baseline_unix_ms: Option<u64>) -> Self {
        Self {
            session: None,
            latest_running_turn: None,
            last_terminal_turn: None,
            baseline_unix_ms,
        }
    }

    /// Record evidence from a non-unique identity without publishing it.
    /// In particular a terminal observed before a later running report for
    /// the same turn remains consumed.
    pub(crate) fn observe_ambiguous(&mut self, report: &OutcomeReport) {
        let Some(turn) = parse_decimal(&report.turn, false) else {
            return;
        };
        match self.session.as_deref() {
            Some(session) if session != report.session => return,
            None => self.session = Some(report.session.clone()),
            _ => {}
        }
        if report.outcome.is_terminal() {
            self.last_terminal_turn =
                Some(self.last_terminal_turn.map_or(turn, |last| last.max(turn)));
        } else {
            self.latest_running_turn =
                Some(self.latest_running_turn.map_or(turn, |last| last.max(turn)));
        }
    }

    /// Accept a fresh terminal outcome exactly once. Running reports establish
    /// the current session but deliberately do not advance the terminal turn:
    /// `running(N)` followed by `terminal(N)` is valid. When polling coalesces
    /// both reports, a terminal newer than the prior coherent snapshot is also
    /// valid. Startup and reconnect terminal snapshots only prime the gate.
    pub(crate) fn observe(
        &mut self,
        report: &OutcomeReport,
        now_unix_ms: u64,
    ) -> Option<AgentOutcome> {
        let turn = parse_decimal(&report.turn, false)?;
        match self.session.as_deref() {
            Some(session) if session != report.session => return None,
            None => self.session = Some(report.session.clone()),
            _ => {}
        }

        if !report.outcome.is_terminal() {
            self.latest_running_turn = Some(
                self.latest_running_turn
                    .map_or(turn, |latest| latest.max(turn)),
            );
            return None;
        }

        if let Some(latest_running_turn) = self.latest_running_turn {
            // A terminal report for an older turn cannot settle the currently
            // running turn, even though terminal ordering is otherwise based
            // strictly on the last terminal report.
            if turn < latest_running_turn {
                return None;
            }
        }

        if self.last_terminal_turn.is_none() {
            if self.latest_running_turn.is_none()
                && !self
                    .baseline_unix_ms
                    .is_some_and(|baseline| report.at_unix_ms > baseline)
            {
                // There is no evidence that this terminal report is newer
                // than our observation window. Never replay cached metadata.
                self.last_terminal_turn = Some(turn);
                return None;
            }
            if !is_fresh_terminal(report.at_unix_ms, now_unix_ms) {
                return None;
            }
            self.last_terminal_turn = Some(turn);
            return Some(report.outcome);
        }

        let last_terminal_turn = self.last_terminal_turn?;
        if turn <= last_terminal_turn || !is_fresh_terminal(report.at_unix_ms, now_unix_ms) {
            return None;
        }

        self.last_terminal_turn = Some(turn);
        Some(report.outcome)
    }
}

fn is_fresh_terminal(at_unix_ms: u64, now_unix_ms: u64) -> bool {
    if at_unix_ms > now_unix_ms {
        at_unix_ms - now_unix_ms <= TERMINAL_MAX_FUTURE_SKEW_MS
    } else {
        now_unix_ms - at_unix_ms <= TERMINAL_MAX_AGE_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tokens(session: &str, turn: &str, outcome: &str, at: &str) -> Value {
        json!({
            "pet_outcome_authority": OUTCOME_AUTHORITY,
            "pet_outcome_session": session,
            "pet_outcome_turn": turn,
            "pet_outcome": outcome,
            "pet_outcome_at": at,
        })
    }

    #[test]
    fn parses_owned_tokens_and_ignores_unrelated_value_size() {
        let session = "a".repeat(64);
        let mut value = tokens(&session, "1", "running", "0");
        value["other_plugin"] = Value::String("x".repeat(1024));
        let report = OutcomeReport::parse_tokens(&value).expect("valid outcome tokens");
        assert_eq!(report.session, session);
        assert_eq!(report.turn, "1");
        assert_eq!(report.outcome, AgentOutcome::Running);
        assert_eq!(report.at_unix_ms, 0);
    }
    #[test]
    fn rejects_bad_authority_keys_and_turns() {
        let session = "a".repeat(64);
        assert!(OutcomeReport::parse_tokens(&json!({
            "pet_outcome_authority": "other:v1",
            "pet_outcome_session": &session,
            "pet_outcome_turn": "1",
            "pet_outcome": "running",
            "pet_outcome_at": "0",
        }))
        .is_none());
        assert!(OutcomeReport::parse_tokens(&tokens(&session, "01", "running", "0")).is_none());
        assert!(OutcomeReport::parse_tokens(&tokens(&session, "0", "running", "0")).is_none());
        assert!(OutcomeReport::parse_tokens(&json!({
            "pet_outcome_authority": OUTCOME_AUTHORITY,
            "pet_outcome_session": &session,
            "pet_outcome_turn": "1",
            "pet_outcome": "running",
            "pet_outcome_at": "0",
            "invalid key": "ignored",
        }))
        .is_none());
    }

    #[test]
    fn primes_startup_then_accepts_same_turn_terminal_once() {
        let session = "a".repeat(64);
        let startup =
            OutcomeReport::parse_tokens(&tokens(&session, "7", "succeeded", "1000")).unwrap();
        let running =
            OutcomeReport::parse_tokens(&tokens(&session, "8", "running", "1001")).unwrap();
        let terminal =
            OutcomeReport::parse_tokens(&tokens(&session, "8", "failed", "10000")).unwrap();
        let mut tracker = OutcomeTracker::new(None);

        assert_eq!(tracker.observe(&startup, 10_000), None);
        assert_eq!(tracker.observe(&running, 10_000), None);
        assert_eq!(
            tracker.observe(&terminal, 10_000),
            Some(AgentOutcome::Failed)
        );
        assert_eq!(tracker.observe(&terminal, 10_000), None);
    }

    #[test]
    fn accepts_terminal_for_running_turn_and_rejects_older_running_turn() {
        let session = "b".repeat(64);
        let running =
            OutcomeReport::parse_tokens(&tokens(&session, "10", "running", "10000")).unwrap();
        let terminal =
            OutcomeReport::parse_tokens(&tokens(&session, "10", "succeeded", "10000")).unwrap();
        let newer_running =
            OutcomeReport::parse_tokens(&tokens(&session, "12", "running", "10000")).unwrap();
        let older_terminal =
            OutcomeReport::parse_tokens(&tokens(&session, "11", "failed", "10000")).unwrap();
        let mut tracker = OutcomeTracker::new(None);

        assert_eq!(tracker.observe(&running, 10_000), None);
        assert_eq!(
            tracker.observe(&terminal, 10_000),
            Some(AgentOutcome::Succeeded)
        );
        assert_eq!(tracker.observe(&newer_running, 10_000), None);
        assert_eq!(tracker.observe(&older_terminal, 10_000), None);
    }

    #[test]
    fn ambiguous_terminal_stays_consumed_after_same_turn_running() {
        let session = "a".repeat(64);
        let terminal =
            OutcomeReport::parse_tokens(&tokens(&session, "7", "succeeded", "14000")).unwrap();
        let running =
            OutcomeReport::parse_tokens(&tokens(&session, "7", "running", "14000")).unwrap();
        let next = OutcomeReport::parse_tokens(&tokens(&session, "8", "failed", "14000")).unwrap();
        let mut tracker = OutcomeTracker::new(Some(10_000));
        tracker.observe_ambiguous(&terminal);
        tracker.observe_ambiguous(&running);
        assert_eq!(tracker.observe(&terminal, 10_000), None);
        assert_eq!(tracker.observe(&next, 10_000), Some(AgentOutcome::Failed));

        let mut running_only = OutcomeTracker::new(None);
        running_only.observe_ambiguous(&running);
        assert_eq!(
            running_only.observe(&terminal, 10_000),
            Some(AgentOutcome::Succeeded)
        );
        assert_eq!(running_only.observe(&terminal, 10_000), None);
    }

    #[test]
    fn rejects_stale_future_and_other_session_terminal_reports() {
        let session = "c".repeat(64);
        let other_session = "d".repeat(64);
        let startup =
            OutcomeReport::parse_tokens(&tokens(&session, "1", "succeeded", "100")).unwrap();
        let stale = OutcomeReport::parse_tokens(&tokens(&session, "2", "failed", "100")).unwrap();
        let future =
            OutcomeReport::parse_tokens(&tokens(&session, "3", "cancelled", "45001")).unwrap();
        let other =
            OutcomeReport::parse_tokens(&tokens(&other_session, "4", "failed", "10000")).unwrap();
        let mut tracker = OutcomeTracker::new(None);

        assert_eq!(tracker.observe(&startup, 10_000), None);
        assert_eq!(tracker.observe(&stale, 40_000), None);
        assert_eq!(tracker.observe(&future, 40_000), None);
        assert_eq!(tracker.observe(&other, 40_000), None);
    }
}
