//! Timer ala Pomodoro: fokus -> istirahat pendek, tiap N sesi -> istirahat panjang.
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Focus,
    Break,
    LongBreak,
}

pub struct Pomo {
    pub work: u32,  // menit
    pub brk: u32,   // menit
    pub long: u32,  // menit
    pub every: u32, // istirahat panjang tiap N sesi fokus
    pub phase: Phase,
    pub done: u32, // sesi fokus selesai
    end: Option<Instant>,
    paused_left: Option<Duration>,
}

impl Pomo {
    pub fn new(work: u32, brk: u32, long: u32, every: u32) -> Self {
        Pomo { work, brk, long, every, phase: Phase::Focus, done: 0, end: None, paused_left: None }
    }

    pub fn phase_len(&self) -> Duration {
        let m = match self.phase {
            Phase::Focus => self.work,
            Phase::Break => self.brk,
            Phase::LongBreak => self.long,
        };
        Duration::from_secs(m as u64 * 60)
    }

    pub fn running(&self) -> bool {
        self.end.is_some()
    }

    /// Sedang jalan atau dijeda di tengah sesi.
    pub fn active(&self) -> bool {
        self.end.is_some() || self.paused_left.is_some()
    }

    pub fn left(&self) -> Duration {
        match self.end {
            Some(e) => e.saturating_duration_since(Instant::now()),
            None => self.paused_left.unwrap_or_else(|| self.phase_len()),
        }
    }

    pub fn progress(&self) -> f32 {
        let len = self.phase_len().as_secs_f32().max(1.0);
        (1.0 - self.left().as_secs_f32() / len).clamp(0.0, 1.0)
    }

    pub fn toggle(&mut self) {
        if self.end.is_some() {
            self.paused_left = Some(self.left());
            self.end = None;
        } else {
            self.end = Some(Instant::now() + self.left());
            self.paused_left = None;
        }
    }

    pub fn reset(&mut self) {
        *self = Pomo::new(self.work, self.brk, self.long, self.every);
    }

    fn advance(&mut self) {
        if self.phase == Phase::Focus {
            self.done += 1;
            self.phase = if self.done % self.every.max(1) == 0 { Phase::LongBreak } else { Phase::Break };
        } else {
            self.phase = Phase::Focus;
        }
    }

    /// Lewati sesi sekarang; tetap jalan kalau sebelumnya jalan.
    pub fn skip(&mut self) {
        let was = self.running();
        self.advance();
        self.paused_left = None;
        self.end = was.then(|| Instant::now() + self.phase_len());
    }

    /// Panggil berkala. Mengembalikan pesan saat ganti fase (sesi berikutnya otomatis mulai).
    pub fn tick(&mut self) -> Option<String> {
        let e = self.end?;
        if Instant::now() < e {
            return None;
        }
        self.advance();
        self.end = Some(Instant::now() + self.phase_len());
        Some(match self.phase {
            Phase::Focus => format!("Focus · {} min", self.work),
            Phase::Break => format!("Break · {} min", self.brk),
            Phase::LongBreak => format!("Long break · {} min", self.long),
        })
    }

    pub fn label(&self) -> &'static str {
        match self.phase {
            Phase::Focus => "Focus",
            Phase::Break => "Break",
            Phase::LongBreak => "Long break",
        }
    }

    /// Ubah pengaturan: 0 = fokus, 1 = istirahat, 2 = istirahat panjang, 3 = tiap N sesi.
    pub fn adjust(&mut self, field: usize, delta: i32) {
        let step = |v: u32, lo: u32, hi: u32| (v as i32 + delta).clamp(lo as i32, hi as i32) as u32;
        match field {
            0 => self.work = step(self.work, 1, 180),
            1 => self.brk = step(self.brk, 1, 60),
            2 => self.long = step(self.long, 1, 90),
            _ => self.every = step(self.every, 1, 12),
        }
        // fase yang belum dimulai ikut panjang baru
        if !self.active() {
            self.paused_left = None;
        }
    }
}
