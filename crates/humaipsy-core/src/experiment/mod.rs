//! Pure trial-sequencing and counterbalancing logic for a HumaiPsy experiment.
//!
//! Deliberately has no dependency on process-global state or the UI layer: every
//! function here takes its inputs as arguments and returns a value, which is what
//! makes it possible to unit test the counterbalancing math directly (see the
//! `tests` module below) instead of only being able to exercise it by clicking
//! through the app.
//!
//! HumaiPsy currently supports exactly two named conditions (Cooperative and
//! Argumentative), each mapped to its own system-prompt override on `HumaiPsy`. A
//! design's requested condition count is clamped to [`MAX_CONDITIONS`] rather than
//! silently ignored, so the Design tab's "Conditions" control is honest about what
//! it can actually do — see `ARCHITECTURE.md`'s roadmap for generalizing this to an
//! arbitrary, researcher-authored set of conditions.

/// The number of named conditions HumaiPsy currently supports. A design's
/// `conditions` field is clamped to this range (`1..=MAX_CONDITIONS`), never
/// silently ignored.
pub const MAX_CONDITIONS: usize = 2;

/// Stable, order-significant condition names. Index 0 is "Cooperative", index 1
/// is "Argumentative" — this order is load-bearing for [`counterbalance_starts_with_argum`].
pub const CONDITION_NAMES: [&str; MAX_CONDITIONS] = ["COOP", "ARGUM"];

/// Clamp a researcher-provided condition count to what HumaiPsy can actually run.
pub fn clamp_condition_count(requested: u32) -> usize {
    (requested as usize).clamp(1, MAX_CONDITIONS)
}

/// Clamp a researcher-provided per-condition trial count to a sane minimum.
pub fn clamp_trials_per_condition(requested: u32) -> usize {
    (requested as usize).max(1)
}

/// Total number of trial windows across the whole experiment
/// (`conditions × trials_per_condition`), both already clamped to sane ranges.
pub fn total_experiment_trials(conditions: usize, trials_per_condition: usize) -> usize {
    conditions.clamp(1, MAX_CONDITIONS) * trials_per_condition.max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrialDescriptor {
    pub condition_index: usize,
    pub trial_in_condition: usize,
}

/// Extracts a numeric value from a participant ID for counterbalancing purposes:
/// the whole ID if purely numeric, otherwise its trailing run of digits (e.g.
/// "P042" -> 42), otherwise every digit in the ID concatenated (e.g. "P4A2" ->
/// 42), otherwise `None` if the ID has no digits at all.
pub fn participant_id_numeric_value(participant_id: &str) -> Option<u64> {
    let trimmed = participant_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = trimmed.parse::<u64>() {
        return Some(value);
    }

    let trailing_digits = trimmed
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if !trailing_digits.is_empty() {
        return trailing_digits.parse().ok();
    }

    let all_digits: String = trimmed.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if all_digits.is_empty() {
        None
    } else {
        all_digits.parse().ok()
    }
}

/// Odd-numbered participant IDs start on the Argumentative condition instead of
/// Cooperative, so consecutive participants counterbalance condition order.
/// IDs with no extractable digit default to starting on Cooperative (`false`).
pub fn counterbalance_starts_with_argum(participant_id: &str) -> bool {
    participant_id_numeric_value(participant_id)
        .map(|value| value % 2 == 1)
        .unwrap_or(false)
}

/// Maps a flat trial index (0-based, across the whole experiment) to a
/// condition + trial-in-condition pair, applying counterbalancing when there is
/// more than one condition. `conditions` is clamped to `1..=MAX_CONDITIONS`.
pub fn trial_descriptor(index: usize, participant_id: &str, conditions: usize) -> TrialDescriptor {
    let condition_count = conditions.clamp(1, MAX_CONDITIONS);
    let starts_with_argum = condition_count > 1 && counterbalance_starts_with_argum(participant_id);
    let base_condition = index % condition_count;
    let condition_index = if starts_with_argum {
        (base_condition + 1) % condition_count
    } else {
        base_condition
    };
    let trial_in_condition = index / condition_count;
    TrialDescriptor {
        condition_index,
        trial_in_condition,
    }
}

/// The stable condition name ("COOP"/"ARGUM") for a condition index, clamped to
/// a valid entry so an out-of-range index can never panic.
pub fn condition_name(condition_index: usize) -> &'static str {
    CONDITION_NAMES
        .get(condition_index)
        .copied()
        .unwrap_or(CONDITION_NAMES[0])
}

/// Human-readable label for a condition name, used in CSV rows and the UI.
pub fn condition_label(name: &str) -> &'static str {
    match name {
        "ARGUM" => "Argum",
        _ => "Cooperative",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_id_numeric_value_parses_pure_digits() {
        assert_eq!(participant_id_numeric_value("42"), Some(42));
    }

    #[test]
    fn participant_id_numeric_value_falls_back_to_trailing_digits() {
        assert_eq!(participant_id_numeric_value("P042"), Some(42));
        assert_eq!(participant_id_numeric_value("participant-7"), Some(7));
    }

    #[test]
    fn participant_id_numeric_value_falls_back_to_all_digits_when_no_trailing_run() {
        // "4A2B" ends in a non-digit, so there is no trailing digit run at all;
        // every digit in the ID is concatenated instead: "4" + "2" = 42.
        assert_eq!(participant_id_numeric_value("4A2B"), Some(42));
    }

    #[test]
    fn participant_id_numeric_value_is_none_for_no_digits() {
        assert_eq!(participant_id_numeric_value("participant"), None);
        assert_eq!(participant_id_numeric_value(""), None);
        assert_eq!(participant_id_numeric_value("   "), None);
    }

    #[test]
    fn counterbalance_starts_with_argum_is_odd_even_parity() {
        assert!(!counterbalance_starts_with_argum("2"));
        assert!(counterbalance_starts_with_argum("3"));
        assert!(!counterbalance_starts_with_argum("P042")); // 42 is even
        assert!(counterbalance_starts_with_argum("P043")); // 43 is odd
    }

    #[test]
    fn counterbalance_starts_with_argum_defaults_false_for_non_numeric_id() {
        assert!(!counterbalance_starts_with_argum("participant"));
    }

    #[test]
    fn trial_descriptor_even_participant_runs_coop_argum_coop_argum() {
        let seq: Vec<(usize, usize)> = (0..4)
            .map(|i| {
                let d = trial_descriptor(i, "2", 2);
                (d.condition_index, d.trial_in_condition)
            })
            .collect();
        // COOP(trial1), ARGUM(trial1), COOP(trial2), ARGUM(trial2)
        assert_eq!(seq, vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
    }

    #[test]
    fn trial_descriptor_odd_participant_runs_argum_coop_argum_coop() {
        let seq: Vec<(usize, usize)> = (0..4)
            .map(|i| {
                let d = trial_descriptor(i, "3", 2);
                (d.condition_index, d.trial_in_condition)
            })
            .collect();
        // ARGUM(trial1), COOP(trial1), ARGUM(trial2), COOP(trial2)
        assert_eq!(seq, vec![(1, 0), (0, 0), (1, 1), (0, 1)]);
    }

    #[test]
    fn trial_descriptor_respects_configurable_trials_per_condition() {
        // 2 conditions x 5 trials = 10 trial windows; verify the trial-in-condition
        // count actually reaches 5, i.e. the Design tab's "Trials" value is
        // honored rather than silently clamped to the old hardcoded 2.
        let last = trial_descriptor(9, "2", 2);
        assert_eq!(last.trial_in_condition, 4);
        assert_eq!(total_experiment_trials(2, 5), 10);
    }

    #[test]
    fn trial_descriptor_with_single_condition_never_counterbalances() {
        // With only one condition selected, every trial stays on condition 0
        // regardless of participant ID parity.
        for participant in ["2", "3", "P099"] {
            for i in 0..4 {
                let d = trial_descriptor(i, participant, 1);
                assert_eq!(d.condition_index, 0);
            }
        }
    }

    #[test]
    fn condition_count_and_trial_count_are_clamped_not_silently_dropped() {
        assert_eq!(clamp_condition_count(0), 1);
        assert_eq!(clamp_condition_count(1), 1);
        assert_eq!(clamp_condition_count(2), 2);
        assert_eq!(clamp_condition_count(99), MAX_CONDITIONS);
        assert_eq!(clamp_trials_per_condition(0), 1);
        assert_eq!(clamp_trials_per_condition(7), 7);
    }

    #[test]
    fn condition_name_never_panics_out_of_range() {
        assert_eq!(condition_name(0), "COOP");
        assert_eq!(condition_name(1), "ARGUM");
        assert_eq!(condition_name(99), "COOP");
    }

    #[test]
    fn condition_label_maps_known_and_unknown_names() {
        assert_eq!(condition_label("ARGUM"), "Argum");
        assert_eq!(condition_label("COOP"), "Cooperative");
        assert_eq!(condition_label("unknown"), "Cooperative");
    }
}
