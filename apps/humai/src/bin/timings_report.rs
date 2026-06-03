use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};

use anyhow::{Context, Result};
use rand::Rng;
use serde_json::Value;

#[derive(Clone)]
struct InferenceEvent {
    model: String,
    success: bool,
    error_type: String,
    duration_ms: u128,
    ttft_us: u128,
    prompt_tokens: u64,
    candidate_tokens: u64,
    input_chars: u64,
    output_chars: u64,
    turn_index: Option<u32>,
}

#[derive(Default)]
struct ModelAgg {
    inference_count: usize,
    success_count: usize,
    stopped_count: usize,
    error_count: usize,
    durations_ms: Vec<u128>,
    ttft_us: Vec<u128>,
    prompt_tokens: Vec<u64>,
    candidate_tokens: Vec<u64>,
    total_tokens: Vec<u64>,
    input_chars: Vec<u64>,
    output_chars: Vec<u64>,
}

#[derive(Default)]
struct TurnAgg {
    turn_count: usize,
    gap_ms_all: Vec<u128>,
    gap_ms_nonzero: Vec<u128>,
}

fn percentile_u128(values: &[u128], percentile: f64) -> Option<u128> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let idx = ((sorted.len() - 1) as f64 * percentile).round() as usize;
    sorted.get(idx).copied()
}

fn percentile_u64(values: &[u64], percentile: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let idx = ((sorted.len() - 1) as f64 * percentile).round() as usize;
    sorted.get(idx).copied()
}

fn percentile_f64(values: &[f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() - 1) as f64 * percentile).round() as usize;
    sorted.get(idx).copied()
}

fn mean_u128(values: &[u128]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let sum: u128 = values.iter().copied().sum();
    Some(sum as f64 / values.len() as f64)
}

fn mean_u64(values: &[u64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let sum: u128 = values.iter().map(|v| *v as u128).sum();
    Some(sum as f64 / values.len() as f64)
}

fn stddev_u128(values: &[u128]) -> Option<f64> {
    let mean = mean_u128(values)?;
    if values.len() < 2 {
        return None;
    }
    let variance = values
        .iter()
        .map(|v| {
            let diff = *v as f64 - mean;
            diff * diff
        })
        .sum::<f64>()
        / (values.len() - 1) as f64;
    Some(variance.sqrt())
}

fn bootstrap_mean_ci_u128(values: &[u128], reps: usize, alpha: f64) -> Option<(u128, u128)> {
    if values.len() < 2 || reps < 100 {
        return None;
    }
    let n = values.len();
    let mut rng = rand::rng();
    let mut boot_means = Vec::with_capacity(reps);

    for _ in 0..reps {
        let mut sum: u128 = 0;
        for _ in 0..n {
            let idx = rng.random_range(0..n);
            sum += values[idx];
        }
        boot_means.push(sum as f64 / n as f64);
    }

    boot_means.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let lo_idx = ((alpha / 2.0) * reps as f64).floor() as usize;
    let hi_idx = (((1.0 - alpha / 2.0) * reps as f64).ceil() as usize)
        .saturating_sub(1)
        .min(reps - 1);
    Some((boot_means[lo_idx].round() as u128, boot_means[hi_idx].round() as u128))
}

fn parse_u128(v: Option<&Value>) -> Option<u128> {
    v.and_then(|x| x.as_u64()).map(|n| n as u128)
}

fn parse_u64(v: Option<&Value>) -> Option<u64> {
    v.and_then(|x| x.as_u64())
}

fn parse_string(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn is_outlier(value: u128, mean: f64, stddev: f64, z_threshold: f64) -> bool {
    if stddev == 0.0 {
        return false;
    }
    let z_score = ((value as f64 - mean).abs()) / stddev;
    z_score > z_threshold
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let input_path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "metrics/timings.jsonl".to_string());

    let file = File::open(&input_path)
        .with_context(|| format!("failed to open metrics file: {}", input_path))?;
    let reader = BufReader::new(file);

    let mut models: BTreeMap<String, ModelAgg> = BTreeMap::new();
    let mut turn_agg = TurnAgg::default();
    let mut total_lines = 0usize;
    let mut bad_lines = 0usize;
    let mut inference_events: Vec<InferenceEvent> = Vec::new();

    let mut runs_seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut models_per_run: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        total_lines += 1;

        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => {
                bad_lines += 1;
                continue;
            }
        };

        let event_type = parse_string(parsed.get("event_type")).unwrap_or_default();
        match event_type.as_str() {
            "inference_timing" => {
                let model = parse_string(parsed.get("model"))
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or_else(|| "<unknown>".to_string());
                let agg = models.entry(model.clone()).or_default();

                agg.inference_count += 1;

                let success = parsed
                    .get("success")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                let error_type = if success {
                    agg.success_count += 1;
                    "none".to_string()
                } else {
                    let err = parse_string(parsed.get("error")).unwrap_or_default();
                    if err == "ollama inference stopped" {
                        agg.stopped_count += 1;
                    } else {
                        agg.error_count += 1;
                    }
                    err
                };

                let duration_ms = parse_u128(parsed.get("duration_ms")).unwrap_or(0);
                let ttft_us = parse_u128(parsed.get("ttft_us")).unwrap_or(0);
                let prompt_tokens = parse_u64(parsed.get("prompt_token_count")).unwrap_or(0);
                let candidate_tokens = parse_u64(parsed.get("candidates_token_count")).unwrap_or(0);
                let total_tokens = parse_u64(parsed.get("total_token_count")).unwrap_or(0);
                let input_chars = parse_u64(parsed.get("input_chars")).unwrap_or(0);
                let output_chars = parse_u64(parsed.get("output_chars")).unwrap_or(0);

                if duration_ms > 0 {
                    agg.durations_ms.push(duration_ms);
                }
                if ttft_us > 0 {
                    agg.ttft_us.push(ttft_us);
                }
                if prompt_tokens > 0 {
                    agg.prompt_tokens.push(prompt_tokens);
                }
                if candidate_tokens > 0 {
                    agg.candidate_tokens.push(candidate_tokens);
                }
                if total_tokens > 0 {
                    agg.total_tokens.push(total_tokens);
                }
                if input_chars > 0 {
                    agg.input_chars.push(input_chars);
                }
                if output_chars > 0 {
                    agg.output_chars.push(output_chars);
                }

                let turn_index = parsed.get("turn_index").and_then(|x| x.as_u64()).map(|n| n as u32);

                inference_events.push(InferenceEvent {
                    model: model.clone(),
                    success,
                    error_type,
                    duration_ms,
                    ttft_us,
                    prompt_tokens,
                    candidate_tokens,
                    input_chars,
                    output_chars,
                    turn_index,
                });

                let exp = parse_string(parsed.get("experiment_id"));
                let run = parse_string(parsed.get("run_id"));
                if let (Some(exp), Some(run)) = (exp, run) {
                    runs_seen.insert((exp.clone(), run.clone()));
                    models_per_run
                        .entry((exp, run))
                        .or_default()
                        .insert(model);
                }
            }
            "turn_timing" => {
                turn_agg.turn_count += 1;
                if let Some(gap) = parse_u128(parsed.get("gap_ms")) {
                    turn_agg.gap_ms_all.push(gap);
                    if gap > 0 {
                        turn_agg.gap_ms_nonzero.push(gap);
                    }
                }
                let exp = parse_string(parsed.get("experiment_id"));
                let run = parse_string(parsed.get("run_id"));
                if let (Some(exp), Some(run)) = (exp, run) {
                    runs_seen.insert((exp, run));
                }
            }
            _ => {}
        }
    }

    println!("TIMINGS REPORT");
    println!("input_file: {}", input_path);
    println!("json_lines: {}", total_lines);
    println!("bad_lines: {}", bad_lines);
    println!("runs_seen: {}", runs_seen.len());
    println!("models_seen: {}", models.len());
    println!();

    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!("DETAILED INFERENCE EVENT TABLE");
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!(
        "{:<20} {:>8} {:>8} {:>9} {:>9} {:>8} {:>8} {:>11} {:>11} {:>11}",
        "model",
        "dur_ms",
        "ttft_us",
        "out_toks",
        "toks/sec",
        "ms/tok",
        "status",
        "in_chars",
        "out_chars",
        "pr_tokens"
    );
    println!("───────────────────────────────────────────────────────────────────────────────────────────────────────────");

    for evt in &inference_events {
        let tokens_per_sec = if evt.duration_ms > 0 && evt.candidate_tokens > 0 {
            (evt.candidate_tokens as f64 * 1000.0) / evt.duration_ms as f64
        } else {
            0.0
        };

        let ms_per_tok = if evt.candidate_tokens > 0 && evt.duration_ms > 0 {
            evt.duration_ms as f64 / evt.candidate_tokens as f64
        } else {
            0.0
        };

        let status = if evt.success {
            "OK".to_string()
        } else {
            format!("FAIL({})", &evt.error_type[..evt.error_type.len().min(3)])
        };

        println!(
            "{:<20} {:>8} {:>8} {:>9} {:>9.2} {:>8.2} {:>8} {:>11} {:>11} {:>11}",
            evt.model,
            evt.duration_ms,
            evt.ttft_us,
            evt.candidate_tokens,
            tokens_per_sec,
            ms_per_tok,
            status,
            evt.input_chars,
            evt.output_chars,
            evt.prompt_tokens,
        );
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!("MODEL SUMMARY (WITH ADVANCED METRICS)");
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!(
        "{:<24} {:>7} {:>8} {:>8} {:>9} {:>9} {:>9} {:>10} {:>10} {:>10} {:>10} {:>11}",
        "model",
        "calls",
        "ok%",
        "stop%",
        "avg_ms",
        "p50_ms",
        "p95_ms",
        "stddev_ms",
        "ci95_lo",
        "ci95_hi",
        "tok/sec",
        "ms/tok"
    );
    println!("───────────────────────────────────────────────────────────────────────────────────────────────────────────");

    for (model, agg) in &models {
        let calls = agg.inference_count as f64;
        let ok_pct = if calls > 0.0 {
            100.0 * agg.success_count as f64 / calls
        } else {
            0.0
        };
        let stop_pct = if calls > 0.0 {
            100.0 * agg.stopped_count as f64 / calls
        } else {
            0.0
        };

        let avg_ms = mean_u128(&agg.durations_ms).unwrap_or(0.0);
        let p50 = percentile_u128(&agg.durations_ms, 0.50).unwrap_or(0);
        let p95 = percentile_u128(&agg.durations_ms, 0.95).unwrap_or(0);
        let stddev = stddev_u128(&agg.durations_ms).unwrap_or(0.0);
        let ci95 = bootstrap_mean_ci_u128(&agg.durations_ms, 2000, 0.05).unwrap_or((0, 0));

        // Calculate tokens per second (output tokens / duration in seconds)
        let avg_out_tokens = mean_u64(&agg.candidate_tokens).unwrap_or(0.0);
        let tok_per_sec = if avg_ms > 0.0 && avg_out_tokens > 0.0 {
            (avg_out_tokens * 1000.0) / avg_ms
        } else {
            0.0
        };

        // Calculate ms per token
        let ms_per_tok = if avg_out_tokens > 0.0 && avg_ms > 0.0 {
            avg_ms / avg_out_tokens
        } else {
            0.0
        };

        println!(
            "{:<24} {:>7} {:>7.1}% {:>7.1}% {:>9.0} {:>9} {:>9} {:>10.0} {:>10} {:>10} {:>10.2} {:>11.2}",
            model,
            agg.inference_count,
            ok_pct,
            stop_pct,
            avg_ms,
            p50,
            p95,
            stddev,
            ci95.0,
            ci95.1,
            tok_per_sec,
            ms_per_tok,
        );
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!("TTFT AND STATISTICAL DETAILS");
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!(
        "{:<24} {:>10} {:>10} {:>10} {:>12} {:>10} {:>10}",
        "model",
        "ttft_us_avg",
        "ttft_us_p50",
        "ttft_us_p95",
        "ttft_stddev_us",
        "cv_duration",
        "cv_ttft"
    );
    println!("───────────────────────────────────────────────────────────────────────────────────────────────────────────");

    for (model, agg) in &models {
        let ttft_p50 = percentile_u128(&agg.ttft_us, 0.50).unwrap_or(0);
        let ttft_p95 = percentile_u128(&agg.ttft_us, 0.95).unwrap_or(0);
        let ttft_avg = mean_u128(&agg.ttft_us).unwrap_or(0.0);
        let ttft_std = stddev_u128(&agg.ttft_us).unwrap_or(0.0);

        let avg_ms = mean_u128(&agg.durations_ms).unwrap_or(1.0);
        let dur_std = stddev_u128(&agg.durations_ms).unwrap_or(0.0);
        let cv_duration = if avg_ms > 0.0 { dur_std / avg_ms } else { 0.0 };
        let cv_ttft = if ttft_avg > 0.0 { ttft_std / ttft_avg } else { 0.0 };

        println!(
            "{:<24} {:>10.0} {:>10} {:>10} {:>12.0} {:>10.2} {:>10.2}",
            model,
            ttft_avg,
            ttft_p50,
            ttft_p95,
            ttft_std,
            cv_duration,
            cv_ttft,
        );
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!("TURN SUMMARY (INTER-TURN SCHEDULING & DIALOGUE PACING)");
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    
    let gap_count = turn_agg.gap_ms_all.len();
    let nonzero_gap_count = turn_agg.gap_ms_nonzero.len();
    let zero_gap_pct = if gap_count > 0 {
        100.0 * (gap_count - nonzero_gap_count) as f64 / gap_count as f64
    } else {
        0.0
    };
    println!("turn_events: {}", turn_agg.turn_count);
    println!("turn_events_with_gap_ms: {}", gap_count);
    println!("gap_ms_zero_ratio: {:.1}%", zero_gap_pct);
    println!(
        "gap_ms_avg_nonzero: {:.1} ms",
        mean_u128(&turn_agg.gap_ms_nonzero).unwrap_or(0.0)
    );
    println!(
        "gap_ms_p50_nonzero: {} ms",
        percentile_u128(&turn_agg.gap_ms_nonzero, 0.50).unwrap_or(0)
    );
    println!(
        "gap_ms_p95_nonzero: {} ms",
        percentile_u128(&turn_agg.gap_ms_nonzero, 0.95).unwrap_or(0)
    );

    println!();
    println!("RUN -> MODELS");
    for ((exp, run), model_set) in models_per_run {
        let joined = model_set.into_iter().collect::<Vec<_>>().join(", ");
        println!("{} / {} => {}", exp, run, joined);
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!("OUTPUT TOKEN DISTRIBUTION & LATENCY DEGRADATION");
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!(
        "{:<24} {:>8} {:>8} {:>8} {:>8} {:>12} {:>12} {:>10} {:>10} {:>8}",
        "model",
        "out_p50",
        "out_p95",
        "out_max",
        "out_min",
        "toks_p1_turn",
        "toks_p10_turn",
        "toks_p50_turn",
        "toks_p95_turn",
        "outliers%"
    );
    println!("───────────────────────────────────────────────────────────────────────────────────────────────────────────");

    for (model, agg) in &models {
        // Output distribution
        let out_p50 = percentile_u64(&agg.candidate_tokens, 0.50).unwrap_or(0);
        let out_p95 = percentile_u64(&agg.candidate_tokens, 0.95).unwrap_or(0);
        let out_max = agg.candidate_tokens.iter().max().copied().unwrap_or(0);
        let out_min = agg.candidate_tokens.iter().min().copied().unwrap_or(0);

        // Latency by turn (only for events with turn_index)
        let mut turn_durations: std::collections::BTreeMap<u32, Vec<u128>> = std::collections::BTreeMap::new();
        for evt in &inference_events {
            if evt.model == *model && evt.success && let Some(turn_idx) = evt.turn_index {
                turn_durations.entry(turn_idx).or_default().push(evt.duration_ms);
            }
        }
        
        let turn_lats: Vec<f64> = turn_durations
            .values()
            .filter_map(|v| mean_u128(v))
            .collect();
        let toks_p1_turn = percentile_f64(&turn_lats, 0.01).unwrap_or(0.0);
        let toks_p10_turn = percentile_f64(&turn_lats, 0.10).unwrap_or(0.0);
        let toks_p50_turn = percentile_f64(&turn_lats, 0.50).unwrap_or(0.0);
        let toks_p95_turn = percentile_f64(&turn_lats, 0.95).unwrap_or(0.0);

        // Outlier detection (>2σ from mean)
        let avg_ms = mean_u128(&agg.durations_ms).unwrap_or(0.0);
        let stddev = stddev_u128(&agg.durations_ms).unwrap_or(0.0);
        let outlier_count = inference_events
            .iter()
            .filter(|evt| evt.model == *model && evt.success && is_outlier(evt.duration_ms, avg_ms, stddev, 2.0))
            .count();
        let outlier_pct = if !agg.durations_ms.is_empty() {
            100.0 * outlier_count as f64 / agg.durations_ms.len() as f64
        } else {
            0.0
        };

        println!(
            "{:<24} {:>8} {:>8} {:>8} {:>8} {:>12.0} {:>12.0} {:>10.0} {:>10.0} {:>7.1}%",
            model,
            out_p50,
            out_p95,
            out_max,
            out_min,
            toks_p1_turn,
            toks_p10_turn,
            toks_p50_turn,
            toks_p95_turn,
            outlier_pct,
        );
    }

    println!();
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");
    println!("LATENCY DEGRADATION BY CONVERSATION TURN");
    println!("═══════════════════════════════════════════════════════════════════════════════════════════════════════════");

    // Build turn-level aggregates for each model
    for (model_name, _) in &models {
        let mut turn_stats: std::collections::BTreeMap<u32, (f64, f64, usize)> = std::collections::BTreeMap::new();
        
        for evt in &inference_events {
            if evt.model == *model_name && evt.success && let Some(turn_idx) = evt.turn_index {
                let entry = turn_stats.entry(turn_idx).or_insert((0.0, 0.0, 0));
                entry.0 += evt.duration_ms as f64;
                entry.2 += 1;
            }
        }

        if turn_stats.is_empty() {
            continue;
        }

        println!("\n  {}", model_name);
        println!("  {:>6} {:>10} {:>10} {:>8}", "turn", "avg_ms", "delta_ms", "n_infer");
        println!("  {}", "───────────────────────────────");

        let mut prev_avg = 0.0;
        for (turn_idx, (total_ms, _, count)) in turn_stats.iter() {
            let avg = total_ms / *count as f64;
            let delta = avg - prev_avg;
            println!("  {:>6} {:>10.0} {:>10.0} {:>8}", turn_idx, avg, delta, count);
            prev_avg = avg;
        }
    }

    println!();
    println!("NOTES");
    println!("- Large avg_out_tk often correlates with higher avg_ms.");
    println!("- Persistent stop% indicates user-driven interruptions (not model failures).");
    println!("- If gap_ms_zero_ratio is near 100%, gap timing may be too coarse to diagnose scheduling stalls.");
    println!("- outliers% = events with latency >2σ from model mean (may indicate GC, context switch, or resource contention).");
    println!("- Latency degradation by turn shows context length effect: increasing turn_index = growing conversational history.");

    Ok(())
}
