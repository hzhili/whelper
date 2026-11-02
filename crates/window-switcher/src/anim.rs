//! 轻量时间轴：只做“进度 + 缓动”，由覆盖窗口的 `WM_TIMER` 驱动。
//!
//! 把时间来源交给调用方（`prod(now)` 而不是内部读时钟），因此可以精确单测；
//! 覆盖窗口每帧把同一份 `now` 传给所有动画，避免同一帧里各动画进度不一致。

use std::time::{Duration, Instant};

/// 一个从 `start` 起算、持续 `duration` 的时间轴。
#[derive(Debug, Clone, Copy)]
pub struct Anim {
    start: Instant,
    duration: Duration,
}

impl Anim {
    pub fn new(now: Instant, duration: Duration) -> Self {
        Self {
            start: now,
            duration,
        }
    }

    /// 线性进度 `0..=1`。`duration` 为 0 时立即完成。
    pub fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        (elapsed / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// 缓动后的进度：界面上的位移/透明度都按这个值插值。
    pub fn eased(&self, now: Instant) -> f32 {
        ease_out_cubic(self.progress(now))
    }

    pub fn is_done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }
}

/// 三次缓出：起步快、收尾慢，适合面板出现与选择移动。
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// 两个透明度之间按 `t` 插值（`SetLayeredWindowAttributes` 只接受 0..=255）。
pub fn lerp_u8(from: u8, to: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (from as f32 + (to as f32 - from as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_clamped_to_the_duration() {
        let now = Instant::now();
        let anim = Anim::new(now, Duration::from_millis(100));
        assert_eq!(anim.progress(now), 0.0);
        assert!((anim.progress(now + Duration::from_millis(50)) - 0.5).abs() < 0.01);
        assert_eq!(anim.progress(now + Duration::from_millis(150)), 1.0);
        assert!(anim.is_done(now + Duration::from_millis(100)));
        assert!(!anim.is_done(now + Duration::from_millis(99)));
    }

    #[test]
    fn zero_duration_finishes_immediately() {
        let now = Instant::now();
        let anim = Anim::new(now, Duration::ZERO);
        assert_eq!(anim.progress(now), 1.0);
        assert!(anim.is_done(now));
    }

    #[test]
    fn time_before_start_is_not_negative() {
        let now = Instant::now();
        let anim = Anim::new(now + Duration::from_millis(50), Duration::from_millis(100));
        assert_eq!(anim.progress(now), 0.0);
    }

    #[test]
    fn ease_out_cubic_has_the_expected_endpoints() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        // 缓出：中点已经走完超过一半。
        assert!(ease_out_cubic(0.5) > 0.5);
        assert!(ease_out_cubic(-1.0).abs() < f32::EPSILON);
        assert_eq!(ease_out_cubic(2.0), 1.0);
    }

    #[test]
    fn lerp_u8_handles_both_directions() {
        assert_eq!(lerp_u8(0, 255, 0.0), 0);
        assert_eq!(lerp_u8(0, 255, 1.0), 255);
        assert_eq!(lerp_u8(0, 255, 0.5), 128);
        assert_eq!(lerp_u8(255, 0, 0.5), 128);
        assert_eq!(lerp_u8(255, 0, 2.0), 0);
    }
}
