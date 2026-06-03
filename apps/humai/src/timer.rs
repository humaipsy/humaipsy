use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static MAIN_TIMER: OnceLock<Mutex<MainTimerState>> = OnceLock::new();

#[derive(Clone, Debug)]
struct MainTimerState {
    running: bool,
    started_at: Option<Instant>,
    accumulated: Duration,
}

impl Default for MainTimerState {
    fn default() -> Self {
        Self {
            running: false,
            started_at: None,
            accumulated: Duration::ZERO,
        }
    }
}

impl MainTimerState {
    fn start(&mut self) {
        if self.running {
            return;
        }
        self.running = true;
        self.started_at = Some(Instant::now());
    }

    fn pause(&mut self) {
        if !self.running {
            return;
        }
        if let Some(started_at) = self.started_at {
            self.accumulated = self.accumulated.saturating_add(started_at.elapsed());
        }
        self.running = false;
        self.started_at = None;
    }

    fn stop_and_reset(&mut self) {
        self.running = false;
        self.started_at = None;
        self.accumulated = Duration::ZERO;
    }

    fn elapsed(&self) -> Duration {
        if self.running {
            if let Some(started_at) = self.started_at {
                return self.accumulated.saturating_add(started_at.elapsed());
            }
        }
        self.accumulated
    }
}

fn format_timer_hms(elapsed: Duration) -> String {
    let total_secs = elapsed.as_secs();
    let hours = total_secs / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub fn main_timer_display() -> String {
    let elapsed = MAIN_TIMER
        .get_or_init(|| Mutex::new(MainTimerState::default()))
        .lock()
        .map(|timer| timer.elapsed())
        .unwrap_or(Duration::ZERO);
    format_timer_hms(elapsed)
}

pub fn main_timer_is_running() -> bool {
    MAIN_TIMER
        .get_or_init(|| Mutex::new(MainTimerState::default()))
        .lock()
        .map(|timer| timer.running)
        .unwrap_or(false)
}

pub fn start_main_timer() {
    if let Ok(mut timer) = MAIN_TIMER
        .get_or_init(|| Mutex::new(MainTimerState::default()))
        .lock()
    {
        timer.start();
    }
}

pub fn stop_and_reset_main_timer() {
    if let Ok(mut timer) = MAIN_TIMER
        .get_or_init(|| Mutex::new(MainTimerState::default()))
        .lock()
    {
        timer.stop_and_reset();
    }
}

pub fn pause_main_timer() {
    if let Ok(mut timer) = MAIN_TIMER
        .get_or_init(|| Mutex::new(MainTimerState::default()))
        .lock()
    {
        timer.pause();
    }
}
