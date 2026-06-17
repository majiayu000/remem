use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhaseTiming {
    pub phase: String,
    pub elapsed_ms: u64,
}

impl PhaseTiming {
    pub fn elapsed(phase: impl Into<String>, start: Instant) -> Self {
        Self {
            phase: phase.into(),
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }
}

pub fn time_result<T>(
    timings: &mut Vec<PhaseTiming>,
    phase: &'static str,
    f: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let start = Instant::now();
    let result = f();
    timings.push(PhaseTiming::elapsed(phase, start));
    result
}

pub fn time_value<T>(
    timings: &mut Vec<PhaseTiming>,
    phase: &'static str,
    f: impl FnOnce() -> T,
) -> T {
    let start = Instant::now();
    let value = f();
    timings.push(PhaseTiming::elapsed(phase, start));
    value
}

pub fn push_elapsed(timings: &mut Vec<PhaseTiming>, phase: &'static str, start: Instant) {
    timings.push(PhaseTiming::elapsed(phase, start));
}

pub fn format_phase_timings(timings: &[PhaseTiming]) -> String {
    timings
        .iter()
        .map(|timing| format!("{}={}ms", timing.phase, timing.elapsed_ms))
        .collect::<Vec<_>>()
        .join(" ")
}
