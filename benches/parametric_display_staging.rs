//! Parametric-chain staging budget benchmark (API v14).
//!
//! Measures the numbers the parametric-display contract is budgeted
//! against:
//!
//! 1. `build_display_entities` for a 50-segment welded chain — short lines
//!    alternating with gentle (R=-3000, L=40) clothoids, seams exact by
//!    construction, the staged footprint under the
//!    MAX_PLUGIN_LIVE_ENTITIES ceiling — at the production tolerance
//!    1e-3 m. The commission's budget: under 5 ms.
//! 2. The same builder on a chord-heavy fixture (five clothoid pairs at
//!    R=-300, L=60, ~200 entities) — the per-entity chord cost.
//! 3. `sample_chain`'s per-call latency over the welded chain (the
//!    placement/basis-curve read path). The commission's budget: under
//!    1 microsecond per sample.
//!
//! Deterministic: fixed fixtures, no randomness. Run:
//! `cargo bench --bench parametric_display_staging`

use std::hint::black_box;
use std::time::{Duration, Instant};

use ocs_plugin_api::host::{
    HorizontalSegmentSpecies, MAX_PLUGIN_LIVE_ENTITIES, ParametricChain, ParametricPlane,
    ParametricSegment, ParametricSpecies,
};
use OpenCADStudio::parametric::{
    build_display_entities, commission_demo_plan_chain, commission_demo_profile_chain,
    sample_chain,
};

const RUNS: usize = 5;
const WARMUP: usize = 2;
const SAMPLES: usize = 2000;
const WELDED_PAIRS: usize = 25;
const CHORD_PAIRS: usize = 5;

fn stepped(point: [f64; 2], direction: f64, length: f64) -> [f64; 2] {
    [
        point[0] + length * direction.cos(),
        point[1] + length * direction.sin(),
    ]
}

fn push_line(segments: &mut Vec<ParametricSegment>, cursor: &mut [f64; 2], heading: f64, length: f64) {
    segments.push(ParametricSegment {
        species: ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Line),
        start_point: *cursor,
        start_direction: heading,
        start_gradient: 0.0,
        end_gradient: 0.0,
        length,
        start_radius: 0.0,
        end_radius: 0.0,
        end_curvature: 0.0,
        bank_change: 0.0,
        gravity_height: 0.0,
    });
    *cursor = stepped(*cursor, heading, length);
}

fn clothoid(
    r: f64,
    cursor: [f64; 2],
    heading: f64,
    length: f64,
) -> (ParametricSegment, f64, [f64; 2]) {
    let segment = ParametricSegment {
        species: ParametricSpecies::Horizontal(HorizontalSegmentSpecies::Clothoid),
        start_point: cursor,
        start_direction: heading,
        start_gradient: 0.0,
        end_gradient: 0.0,
        length,
        start_radius: 0.0,
        end_radius: r,
        end_curvature: 0.0,
        bank_change: 0.0,
        gravity_height: 0.0,
    };
    let dtheta = length / (2.0 * r);
    let end = {
        // The Fresnel series places the end (the engine's own oracle math).
        let b = 1.0 / (2.0 * r * length);
        fresnel_end(b, length)
    };
    (
        segment,
        heading + dtheta,
        [
            cursor[0] + end.0 * heading.cos() - end.1 * heading.sin(),
            cursor[1] + end.0 * heading.sin() + end.1 * heading.cos(),
        ],
    )
}

fn fresnel_end(b: f64, s: f64) -> (f64, f64) {
    fn factorial(n: usize) -> f64 {
        (1..=n as u64).fold(1u128, |acc: u128, v| acc.saturating_mul(v as u128)) as f64
    }
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for k in 0..12usize {
        let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
        x += sign * b.powi(2 * k as i32)
            * s.powi(4 * k as i32 + 1)
            / (factorial(2 * k) * (4.0 * k as f64 + 1.0));
        y += sign * b.powi(2 * k as i32 + 1)
            * s.powi(4 * k as i32 + 3)
            / (factorial(2 * k + 1) * (4.0 * k as f64 + 3.0));
    }
    (x, y)
}

/// The welded 50-segment chain (lines + gentle clothoids), seams exact.
fn welded_chain(r: f64, line_length: f64, clothoid_length: f64, pairs: usize) -> ParametricChain {
    let mut segments = Vec::new();
    let mut cursor = [0.0f64, 0.0];
    let mut heading = 0.0f64;
    for _ in 0..pairs {
        push_line(&mut segments, &mut cursor, heading, line_length);
        let (segment, next_heading, next_cursor) = clothoid(r, cursor, heading, clothoid_length);
        segments.push(segment);
        cursor = next_cursor;
        heading = next_heading;
    }
    ParametricChain {
        plane: ParametricPlane::Plan,
        segments,
        layer: String::new(),
    }
}

fn main() {
    // The demo fixtures the API documents stage.
    let (entities, _) = build_display_entities(&commission_demo_plan_chain(), 1e-3).expect("demo plan stages");
    assert!(!entities.is_empty());
    let _ = build_display_entities(&commission_demo_profile_chain(), 1e-3).expect("demo profile stages");

    // Budget 1: the 50-segment welded chain (under the cap), < 5 ms.
    let chain = welded_chain(-3000.0, 2.0, 40.0, WELDED_PAIRS);
    let (entities, _) = build_display_entities(&chain, 1e-3).expect("welded chain stages");
    let entity_count = entities.len();
    assert!(
        entity_count <= ocs_plugin_api::host::MAX_PLUGIN_LIVE_ENTITIES,
        "fixture must fit under the live cap, got {entity_count}"
    );
    for _ in 0..WARMUP {
        black_box(build_display_entities(&chain, 1e-3)).expect("warmup stages");
    }
    let mut staging_total = Duration::ZERO;
    for _ in 0..RUNS {
        let start = Instant::now();
        black_box(build_display_entities(&chain, 1e-3)).expect("stages");
        staging_total += start.elapsed();
    }
    let staging_mean = staging_total / RUNS as u32;
    println!("staging: welded {WELDED_PAIRS}-pair chain -> {entity_count} display entities");
    println!("staging: mean {staging_mean:?} over {RUNS} runs (budget < 5 ms)");
    assert!(
        staging_mean < Duration::from_millis(5),
        "staging budget exceeded: {staging_mean:?}"
    );

    // Budget 2: the chord-heavy fixture (~200 entities at 1e-3).
    let chord_chain = welded_chain(-300.0, 2.0, 60.0, CHORD_PAIRS);
    let (entities, _) = build_display_entities(&chord_chain, 1e-3).expect("chord-heavy chain stages");
    let chord_count = entities.len();
    for _ in 0..WARMUP {
        black_box(build_display_entities(&chord_chain, 1e-3)).expect("warmup stages");
    }
    let start = Instant::now();
    black_box(build_display_entities(&chord_chain, 1e-3)).expect("stages");
    println!("staging (chord-heavy {CHORD_PAIRS}-pair, {chord_count} entities): {:?}", start.elapsed());

    // Budget 3: sample latency under 1 us.
    let total: f64 = chain.segments.iter().map(|s| s.length).sum();
    for _ in 0..WARMUP {
        black_box(sample_chain(&chain, total * 0.5).unwrap());
    }
    let start = Instant::now();
    let mut acc = 0.0f64;
    for i in 0..SAMPLES {
        let s = total * i as f64 / SAMPLES as f64;
        if let Some(sample) = sample_chain(black_box(&chain), s).unwrap() {
            acc += sample.point[0] + sample.point[1];
        }
    }
    let elapsed = start.elapsed();
    let per_sample = elapsed / SAMPLES as u32;
    println!("sample: {SAMPLES} calls in {elapsed:?} -> {per_sample:?} per call");
    println!("sample: digest {acc:.3} over {total} m of chain");
    black_box(acc);
}
