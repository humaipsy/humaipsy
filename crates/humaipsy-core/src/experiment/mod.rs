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
//!
//! Condition *order* is one of three [`ConditionOrder`] modes: two fixed
//! alternating sequences, or permuted-block randomisation seeded per participant
//! ([`randomisation_seed`]). Every mode is a pure function of
//! `(index, conditions, order, seed)`, so the UI can recompute the current trial's
//! condition on every frame — and re-derive a participant's order after a restart —
//! without ever relabelling a trial that has already been recorded.
//!
//! The randomisation draws from [`splitmix64`], implemented here rather than taken
//! from `rand`: `rand::rngs::StdRng`'s algorithm is explicitly not stable across
//! `rand` versions, and an assignment order that silently changed when the app was
//! rebuilt with a newer dependency would corrupt a study already in progress.

/// splitmix64's 64-bit finaliser (Steele, Lea & Flood, 2014). Spreads the weak high
/// bits of an FNV hash, and turns a counter step into a well-distributed value.
fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// One splitmix64 step: advance the counter, then mix it.
///
/// This is assignment, not cryptography — there is no adversary to defend against,
/// only a need for an unpredictable, balanced, and reproducible sequence. The
/// algorithm is a published one and is written out here so the exact sequence a
/// study saw can never change underneath it.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    mix64(*state)
}

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
    trial_descriptor_with_start(
        index,
        counterbalance_starts_with_argum(participant_id),
        conditions,
    )
}

/// Like [`trial_descriptor`], but the starting condition is chosen explicitly
/// by the researcher instead of being derived from the participant ID.
///
/// `starts_with_argum` starts the sequence on the Argumentative condition when
/// there are two conditions; with a single condition it has no effect.
pub fn trial_descriptor_with_start(
    index: usize,
    starts_with_argum: bool,
    conditions: usize,
) -> TrialDescriptor {
    let condition_count = conditions.clamp(1, MAX_CONDITIONS);
    let base_condition = index % condition_count;
    let condition_index = if condition_count > 1 && starts_with_argum {
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

/// How a design orders its conditions across the experiment's trial windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConditionOrder {
    /// Fixed alternating order, starting on condition 0 (Cooperative).
    #[default]
    Condition1First,
    /// Fixed alternating order, starting on condition 1 (Argumentative). With a
    /// single condition this has no effect, same as
    /// [`trial_descriptor_with_start`].
    Condition2First,
    /// Permuted-block randomisation: one trial of every condition per block, the
    /// order inside each block drawn from a per-participant seed — see
    /// [`randomised_condition_sequence`].
    Random,
}

/// The condition sequence for one design, dispatching on the researcher's chosen
/// [`ConditionOrder`]. This is the single place where "Condition 1", "Condition 2"
/// and "Random" acquire a meaning, so the three modes cannot drift apart as the
/// UI and the recorder both go through it.
///
/// `seed` is only read by [`ConditionOrder::Random`]; callers that never select it
/// can pass anything (see [`randomisation_seed`] for the value the UI uses).
pub fn trial_descriptor_for_order(
    index: usize,
    conditions: usize,
    order: ConditionOrder,
    seed: u64,
) -> TrialDescriptor {
    match order {
        ConditionOrder::Condition1First => trial_descriptor_with_start(index, false, conditions),
        ConditionOrder::Condition2First => trial_descriptor_with_start(index, true, conditions),
        ConditionOrder::Random => randomised_trial_descriptor(index, conditions, seed),
    }
}

/// A stable 64-bit seed for one participant's randomisation.
///
/// Derived from the participant ID plus the design name: the same ID always draws
/// the same order (so a run is resumable and auditable, and two recordings of one
/// participant agree), while different IDs draw different orders — which is the
/// random assignment a randomised controlled trial relies on. Including the design
/// name keeps two designs that reuse participant numbering from sharing one order
/// table.
///
/// FNV-1a plus a splitmix64 finaliser rather than `std::hash::DefaultHasher`: the
/// standard library explicitly does not promise a stable hash across releases, and
/// an order that silently changes after a toolchain upgrade would corrupt a study
/// mid-run.
pub fn randomisation_seed(participant_id: &str, design_name: &str) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    /// Unit separator: keeps ("P1", "Exp2") from hashing the same bytes as
    /// ("P1E", "xp2").
    const FIELD_SEPARATOR: u8 = 0x1f;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in participant_id
        .trim()
        .bytes()
        .chain(std::iter::once(FIELD_SEPARATOR))
        .chain(design_name.trim().bytes())
    {
        hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
    }

    // FNV leaves the high bits weak, and the shuffle reads the whole word.
    mix64(hash)
}

/// The condition indices of block `block_index`, shuffled — one block being one
/// trial of every condition.
///
/// Each block gets its own stream: the `(seed, block)` pair is mixed into a fresh
/// mixer state, so two blocks of one participant are not adjacent points on a
/// single stream, and a block can be derived without walking the blocks before it.
/// [`randomised_condition_sequence`] and [`randomised_trial_descriptor`] both call
/// this, which is what makes them agree about every trial by construction.
fn block_permutation(condition_count: usize, block_index: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..condition_count).collect();
    let mut state = mix64(seed ^ mix64(block_index as u64));

    // Fisher-Yates. With the two conditions HumaiPsy supports this is exactly one
    // coin flip per block, and `2^64` is divisible by both `1` and `2`, so the
    // modulo is unbiased at these sizes.
    for i in (1..condition_count).rev() {
        let j = (splitmix64(&mut state) % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order
}

/// One trial window under permuted-block randomisation.
///
/// The whole sequence this belongs to is [`randomised_condition_sequence`]; this
/// derives a single window without materialising the rest, which is what the UI
/// needs several times per frame.
pub fn randomised_trial_descriptor(index: usize, conditions: usize, seed: u64) -> TrialDescriptor {
    let condition_count = conditions.clamp(1, MAX_CONDITIONS);
    let trial_in_condition = index / condition_count;
    let block = block_permutation(condition_count, trial_in_condition, seed);
    TrialDescriptor {
        condition_index: block[index % condition_count],
        trial_in_condition,
    }
}

/// The condition index for every trial window of a permuted-block ("randomised
/// block") design, in the order the participant experiences them.
///
/// A block is one trial of every condition, so every condition still appears
/// exactly `trials_per_condition` times — a free shuffle over all windows would
/// let one condition end up with more trials than the design asked for, and a
/// plain alternation has no randomness at all. Shuffling within each block is the
/// same restricted randomisation a randomised controlled trial uses: the order is
/// unpredictable to the participant and the researcher running it, while every
/// block stays balanced, so a session cut short still leaves comparable amounts of
/// each condition.
///
/// Deterministic for a given `(conditions, trials_per_condition, seed)`, which is
/// what makes a participant's order reproducible from their ID. The draw is fixed
/// by this crate's own code, so upgrading a dependency or the toolchain cannot
/// redraw a study's orders.
pub fn randomised_condition_sequence(
    conditions: usize,
    trials_per_condition: usize,
    seed: u64,
) -> Vec<usize> {
    let condition_count = conditions.clamp(1, MAX_CONDITIONS);
    let trials = trials_per_condition.max(1);
    let mut sequence = Vec::with_capacity(condition_count * trials);
    for block_index in 0..trials {
        sequence.extend(block_permutation(condition_count, block_index, seed));
    }
    sequence
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
    fn trial_descriptor_with_start_can_begin_on_argum() {
        let seq: Vec<usize> = (0..4)
            .map(|i| trial_descriptor_with_start(i, true, 2).condition_index)
            .collect();
        // ARGUM, COOP, ARGUM, COOP
        assert_eq!(seq, vec![1, 0, 1, 0]);
    }

    #[test]
    fn trial_descriptor_with_start_ignores_the_start_flag_for_one_condition() {
        for i in 0..4 {
            let d = trial_descriptor_with_start(i, true, 1);
            assert_eq!(d.condition_index, 0);
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

    #[test]
    fn condition_order_defaults_to_the_fixed_condition_1_sequence() {
        assert_eq!(ConditionOrder::default(), ConditionOrder::Condition1First);
    }

    #[test]
    fn trial_descriptor_for_order_matches_the_two_fixed_modes() {
        // The new dispatch must reproduce the pre-existing sequences exactly:
        // this is what "keeping everything working as it is now" means for the
        // two options that shipped first.
        for index in 0..6 {
            assert_eq!(
                trial_descriptor_for_order(index, 2, ConditionOrder::Condition1First, 0),
                trial_descriptor_with_start(index, false, 2)
            );
            assert_eq!(
                trial_descriptor_for_order(index, 2, ConditionOrder::Condition2First, 0),
                trial_descriptor_with_start(index, true, 2)
            );
        }
    }

    #[test]
    fn trial_descriptor_for_order_ignores_the_order_for_one_condition() {
        for order in [
            ConditionOrder::Condition1First,
            ConditionOrder::Condition2First,
            ConditionOrder::Random,
        ] {
            for index in 0..4 {
                let descriptor = trial_descriptor_for_order(index, 1, order, 12345);
                assert_eq!(descriptor.condition_index, 0);
                assert_eq!(descriptor.trial_in_condition, index);
            }
        }
    }

    #[test]
    fn randomisation_seed_is_stable_for_one_participant() {
        // Pinned to a literal: the whole point of this hash is that the same
        // participant keeps their order across runs and upgrades, so changing the
        // algorithm has to be a deliberate, visible edit to this test.
        assert_eq!(
            randomisation_seed("P042", "Exp001"),
            3_729_764_469_142_153_995
        );
        assert_eq!(
            randomisation_seed("P042", "Exp001"),
            randomisation_seed("P042", "Exp001")
        );
    }

    #[test]
    fn randomisation_seed_separates_participants_and_designs() {
        let base = randomisation_seed("P042", "Exp001");
        assert_ne!(base, randomisation_seed("P043", "Exp001"));
        assert_ne!(base, randomisation_seed("P042", "Exp002"));
        // Trimming is part of the contract: the UI passes an already-trimmed ID,
        // so a stray space must not silently fork the order.
        assert_eq!(base, randomisation_seed("  P042  ", " Exp001 "));
        // ("P1", "Exp2") and ("P1E", "xp2") concatenate to the same bytes; the
        // field separator is what keeps them apart.
        assert_ne!(
            randomisation_seed("P1", "Exp2"),
            randomisation_seed("P1E", "xp2")
        );
    }

    #[test]
    fn randomised_condition_sequence_is_balanced_per_block() {
        let sequence = randomised_condition_sequence(2, 5, 0xDEAD_BEEF_u64);
        assert_eq!(sequence.len(), 10);
        for block in sequence.chunks(2) {
            let mut conditions = block.to_vec();
            conditions.sort_unstable();
            assert_eq!(
                conditions,
                vec![0, 1],
                "every block must hold one trial of each condition"
            );
        }
        // Balanced overall, so "Trials per condition" is still exactly honoured.
        assert_eq!(sequence.iter().filter(|&&c| c == 0).count(), 5);
        assert_eq!(sequence.iter().filter(|&&c| c == 1).count(), 5);
    }

    #[test]
    fn randomised_condition_sequence_is_deterministic_for_a_seed() {
        assert_eq!(
            randomised_condition_sequence(2, 4, 7),
            randomised_condition_sequence(2, 4, 7)
        );
    }

    #[test]
    fn randomised_condition_sequence_is_pinned_to_a_known_draw() {
        // Pinned for the same reason as the seed above: a participant's order is
        // data. If this changes, an old run and a new run disagree about what
        // "this participant's order" was, so the change has to be deliberate.
        assert_eq!(
            randomised_condition_sequence(2, 4, 7),
            vec![0, 1, 1, 0, 0, 1, 0, 1]
        );
    }

    #[test]
    fn randomised_orders_stay_balanced_across_participants() {
        // A deterministic guard on the mixer's distribution rather than on one
        // draw: across 400 participants the first block should split near 50/50,
        // and most participants should see more than one block order. A mixer that
        // degenerated — all zeros, or one fixed order — fails this loudly.
        let mut first_block = [0usize; 2];
        let mut mixed = 0usize;
        for participant in 0..400 {
            let seed = randomisation_seed(&format!("P{participant:03}"), "Exp001");
            let sequence = randomised_condition_sequence(2, 4, seed);
            first_block[sequence[0]] += 1;
            let distinct: std::collections::BTreeSet<&[usize]> = sequence.chunks(2).collect();
            if distinct.len() > 1 {
                mixed += 1;
            }
        }
        for count in first_block {
            assert!(
                (100..=300).contains(&count),
                "the first block should be near 50/50 across participants: {first_block:?}"
            );
        }
        assert!(
            mixed > 300,
            "most participants should see more than one block order, got {mixed}"
        );
    }

    #[test]
    fn randomised_condition_sequence_varies_across_seeds() {
        // One seed could legitimately be all-Cooperative-first; across 32
        // participants both block orderings must show up.
        let sequences: std::collections::BTreeSet<Vec<usize>> = (0..32)
            .map(|seed| randomised_condition_sequence(2, 2, seed))
            .collect();
        assert!(
            sequences.len() > 1,
            "randomisation must not degenerate to one fixed order: {sequences:?}"
        );
    }

    #[test]
    fn randomised_condition_sequence_with_one_condition_is_all_zeros() {
        assert_eq!(randomised_condition_sequence(1, 3, 99), vec![0, 0, 0]);
    }

    #[test]
    fn randomised_trial_descriptor_agrees_with_the_whole_sequence() {
        // The UI asks for single windows every frame while the sequence is what a
        // recorder or a preview would walk; they read the same RNG stream, so
        // they must never disagree about a trial.
        for seed in [0_u64, 1, 2, 0x0123_4567_89ab_cdef] {
            let sequence = randomised_condition_sequence(2, 4, seed);
            for (index, expected) in sequence.iter().enumerate() {
                let descriptor = randomised_trial_descriptor(index, 2, seed);
                assert_eq!(descriptor.condition_index, *expected);
                assert_eq!(descriptor.trial_in_condition, index / 2);
            }
        }
    }

    #[test]
    fn randomised_trial_descriptor_keeps_trial_counts_when_index_is_past_the_end() {
        // The UI clamps the trial index, but the function must stay sane for any
        // index rather than panicking on a slice.
        let descriptor = randomised_trial_descriptor(37, 2, 5);
        assert_eq!(descriptor.trial_in_condition, 18);
        assert!(descriptor.condition_index < 2);
    }
}
