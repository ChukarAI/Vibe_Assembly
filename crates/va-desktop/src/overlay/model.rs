use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::overlay::raster::{self, OverlayScene};

pub const BRAND_YELLOW: (u8, u8, u8) = (0xF5, 0xB8, 0x00);
pub const CURSOR_OUTLINE: (u8, u8, u8) = (0x14, 0x14, 0x16);
pub const BASE_CURSOR_DIP: f64 = 48.0;
pub const MAX_ACTION_PILL_BYTES: usize = 48;
pub const TRAIL_CAPACITY: usize = 16;
pub const MOVE_DURATION: Duration = Duration::from_millis(300);
pub const PRECLICK_DURATION: Duration = Duration::from_millis(200);
pub const CLICK_DURATION: Duration = Duration::from_millis(90);
pub const TRAIL_DECAY: Duration = Duration::from_millis(180);
pub const TRAIL_IDLE_HIDE: Duration = Duration::from_millis(1000);
pub const MAX_FRAME_RATE: u32 = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

impl ScreenPoint {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    fn lerp(self, other: Self, amount: f64) -> Self {
        Self::new(
            self.x + (other.x - self.x) * amount,
            self.y + (other.y - self.y) * amount,
        )
    }
}

impl std::ops::Mul<f64> for ScreenPoint {
    type Output = Self;

    fn mul(self, scale: f64) -> Self::Output {
        Self::new(self.x * scale, self.y * scale)
    }
}

impl std::ops::Div<f64> for ScreenPoint {
    type Output = Self;

    fn div(self, scale: f64) -> Self::Output {
        Self::new(self.x / scale, self.y / scale)
    }
}

impl std::ops::Add for ScreenPoint {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        Self::new(self.x + other.x, self.y + other.y)
    }
}

impl std::ops::Sub for ScreenPoint {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        Self::new(self.x - other.x, self.y - other.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorBounds {
    pub origin: ScreenPoint,
    pub width: f64,
    pub height: f64,
}

impl MonitorBounds {
    pub fn new(origin: ScreenPoint, width: f64, height: f64) -> Result<Self, String> {
        if !origin.x.is_finite() || !origin.y.is_finite() {
            return Err("monitor origin must be finite".to_string());
        }
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err("monitor dimensions must be finite and positive".to_string());
        }
        if !(origin.x + width).is_finite() || !(origin.y + height).is_finite() {
            return Err("monitor bounds overflow".to_string());
        }
        Ok(Self {
            origin,
            width,
            height,
        })
    }

    fn clamp(self, point: ScreenPoint) -> ScreenPoint {
        ScreenPoint::new(
            point.x.clamp(self.origin.x, self.origin.x + self.width),
            point.y.clamp(self.origin.y, self.origin.y + self.height),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordTransform {
    pub scale_factor: f64,
    pub monitor: MonitorBounds,
}

impl CoordTransform {
    pub fn new(scale_factor: f64, monitor: MonitorBounds) -> Result<Self, String> {
        if !scale_factor.is_finite() || scale_factor <= 0.0 {
            return Err("capture scale factor must be finite and positive".to_string());
        }
        if !(monitor.width / scale_factor).is_finite()
            || !(monitor.height / scale_factor).is_finite()
            || !(BASE_CURSOR_DIP * scale_factor).is_finite()
        {
            return Err("capture scale factor overflows monitor math".to_string());
        }
        Ok(Self {
            scale_factor,
            monitor,
        })
    }

    pub fn screenshot_to_screen(&self, point: ScreenPoint) -> Result<ScreenPoint, String> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err("screenshot coordinates must be finite".to_string());
        }
        let screenshot_width = self.monitor.width / self.scale_factor;
        let screenshot_height = self.monitor.height / self.scale_factor;
        let bounded = ScreenPoint::new(
            point.x.clamp(0.0, screenshot_width),
            point.y.clamp(0.0, screenshot_height),
        );
        Ok(self
            .monitor
            .clamp(self.monitor.origin + bounded * self.scale_factor))
    }

    #[cfg(test)]
    pub fn screen_to_screenshot(&self, point: ScreenPoint) -> Result<ScreenPoint, String> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err("screen coordinates must be finite".to_string());
        }
        let bounded = self.monitor.clamp(point);
        Ok((bounded - self.monitor.origin) / self.scale_factor)
    }

    pub fn cursor_size(&self) -> Result<f64, String> {
        Ok(BASE_CURSOR_DIP * self.scale_factor)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum OverlayCommand {
    MoveTo(ScreenPoint),
    PreClickPulse,
    Click,
    ParkForTyping,
    SetActionPill(String),
    Hide,
    Abort,
}

#[derive(Clone, Debug)]
pub struct KillSwitch(Arc<AtomicBool>);

impl KillSwitch {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn abort(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl Default for KillSwitch {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
struct TrailPoint {
    position: ScreenPoint,
    sampled_at: Duration,
}

#[derive(Clone, Copy, Debug)]
struct Motion {
    from: ScreenPoint,
    to: ScreenPoint,
    started_at: Duration,
    duration: Duration,
}

#[derive(Clone, Copy, Debug)]
struct PreClick {
    center: ScreenPoint,
    started_at: Duration,
    duration: Duration,
}

#[derive(Clone, Copy, Debug)]
struct ClickSquash {
    started_at: Duration,
    duration: Duration,
}

#[derive(Debug)]
pub struct OverlayState {
    pub visible: bool,
    pub reduced_motion: bool,
    pub transform: CoordTransform,
    pub kill_switch: KillSwitch,
    position: ScreenPoint,
    motion: Option<Motion>,
    pre_click: Option<PreClick>,
    click_squash: Option<ClickSquash>,
    trail: Vec<TrailPoint>,
    last_sample_at: Duration,
    action_pill: Option<String>,
    last_frame_at: Option<Duration>,
    dirty: bool,
}

impl OverlayState {
    pub fn new(
        start: ScreenPoint,
        transform: CoordTransform,
        reduced_motion: bool,
        kill_switch: &KillSwitch,
    ) -> Result<Self, String> {
        if !start.x.is_finite() || !start.y.is_finite() {
            return Err("overlay start must be finite".to_string());
        }
        let position = transform.monitor.clamp(start);
        Ok(Self {
            visible: true,
            reduced_motion,
            transform,
            kill_switch: kill_switch.clone(),
            position,
            motion: None,
            pre_click: None,
            click_squash: None,
            trail: Vec::with_capacity(TRAIL_CAPACITY),
            last_sample_at: Duration::ZERO,
            action_pill: None,
            last_frame_at: None,
            dirty: true,
        })
    }

    pub fn action_pill(&self) -> Option<&str> {
        self.action_pill.as_deref()
    }

    pub fn action_pill_anchor(&self) -> Option<ScreenPoint> {
        self.action_pill.as_ref().map(|_| {
            let size = self.transform.cursor_size().unwrap_or(BASE_CURSOR_DIP);
            ScreenPoint::new(self.position.x, self.position.y - size * 1.35)
        })
    }

    pub fn set_action_pill(&mut self, text: &str) -> Result<(), String> {
        if text.len() > MAX_ACTION_PILL_BYTES {
            return Err(format!(
                "action pill exceeds {MAX_ACTION_PILL_BYTES} UTF-8 bytes"
            ));
        }
        self.action_pill = if text.trim().is_empty() {
            None
        } else {
            Some(text.to_string())
        };
        self.dirty = true;
        Ok(())
    }

    pub fn command(&mut self, command: OverlayCommand, now: Duration) -> Result<(), String> {
        if self.kill_switch.is_aborted() {
            self.hide();
            return Err("overlay aborted".to_string());
        }
        match command {
            OverlayCommand::MoveTo(target) => self.move_to(target, now)?,
            OverlayCommand::PreClickPulse => {
                self.pre_click = Some(PreClick {
                    center: self.motion.map_or(self.position, |motion| motion.to),
                    started_at: now,
                    duration: PRECLICK_DURATION,
                });
            }
            OverlayCommand::Click => {
                self.click_squash = Some(ClickSquash {
                    started_at: now,
                    duration: CLICK_DURATION,
                });
                self.pre_click = None;
            }
            OverlayCommand::ParkForTyping => self.park_for_typing(),
            OverlayCommand::SetActionPill(text) => self.set_action_pill(&text)?,
            OverlayCommand::Hide => self.hide(),
            OverlayCommand::Abort => self.abort(),
        }
        Ok(())
    }

    pub fn move_to(&mut self, target: ScreenPoint, now: Duration) -> Result<(), String> {
        if self.kill_switch.is_aborted() {
            self.abort();
            return Err("overlay aborted".to_string());
        }
        if !target.x.is_finite() || !target.y.is_finite() {
            return Err("movement target must be finite".to_string());
        }
        let bounded = self.transform.monitor.clamp(target);
        self.visible = true;
        self.dirty = true;
        if self.position.distance(bounded) <= f64::EPSILON {
            return Ok(());
        }
        self.motion = Some(Motion {
            from: self.position,
            to: bounded,
            started_at: now,
            duration: if self.reduced_motion {
                Duration::from_millis(1)
            } else {
                MOVE_DURATION
            },
        });
        Ok(())
    }

    pub fn park_for_typing(&mut self) {
        if !self.kill_switch.is_aborted() {
            self.motion = None;
        }
    }

    pub fn abort(&mut self) {
        self.kill_switch.abort();
        self.hide();
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.motion = None;
        self.pre_click = None;
        self.click_squash = None;
        self.trail.clear();
        self.action_pill = None;
        self.last_frame_at = None;
        self.dirty = false;
    }

    pub fn advance(&mut self, now: Duration) {
        if self.kill_switch.is_aborted() {
            self.hide();
            return;
        }
        if !self.visible {
            return;
        }
        if let Some(motion) = self.motion {
            let progress = eased_progress(now, motion.started_at, motion.duration);
            self.position = motion.from.lerp(motion.to, progress);
            if progress >= 1.0 {
                self.position = motion.to;
                self.motion = None;
                self.dirty = true;
            }
        }
        if self
            .pre_click
            .is_some_and(|pulse| elapsed(now, pulse.started_at) >= pulse.duration)
        {
            self.pre_click = None;
            self.dirty = true;
        }
        if self
            .click_squash
            .is_some_and(|click| elapsed(now, click.started_at) >= click.duration)
        {
            self.click_squash = None;
            self.dirty = true;
        }
        if !self.reduced_motion {
            self.sample_trail(now);
        }
        let previous_trail_len = self.trail.len();
        self.trail
            .retain(|point| elapsed(now, point.sampled_at) < TRAIL_DECAY);
        if self.trail.len() != previous_trail_len {
            self.dirty = true;
        }
        if self.motion.is_none()
            && self.pre_click.is_none()
            && self.click_squash.is_none()
            && elapsed(now, self.last_sample_at) >= TRAIL_IDLE_HIDE
        {
            self.trail.clear();
        }
    }

    fn sample_trail(&mut self, now: Duration) {
        if self.motion.is_none() && self.trail.is_empty() {
            return;
        }
        if self.motion.is_some() && elapsed(now, self.last_sample_at) < Duration::from_millis(10) {
            return;
        }
        if self
            .trail
            .last()
            .is_some_and(|point| point.position.distance(self.position) <= f64::EPSILON)
        {
            return;
        }
        self.trail.push(TrailPoint {
            position: self.position,
            sampled_at: now,
        });
        if self.trail.len() > TRAIL_CAPACITY {
            self.trail.remove(0);
        }
        self.last_sample_at = now;
    }

    pub fn needs_frame(&self, now: Duration) -> bool {
        if self.kill_switch.is_aborted() || !self.visible {
            return false;
        }
        if self.last_frame_at.is_some_and(|last| {
            elapsed(now, last) < Duration::from_nanos(1_000_000_000 / u64::from(MAX_FRAME_RATE))
        }) {
            return false;
        }
        self.motion.is_some()
            || self.pre_click.is_some()
            || self.click_squash.is_some()
            || self.dirty
            || self
                .trail
                .iter()
                .any(|point| elapsed(now, point.sampled_at) < TRAIL_DECAY)
    }

    pub fn mark_frame_presented(&mut self, now: Duration) {
        if self.visible {
            self.last_frame_at = Some(now);
            self.dirty = false;
        }
    }

    pub fn scene(&self, now: Duration) -> OverlayScene {
        let mut trail = Vec::new();
        if !self.reduced_motion && self.visible && !self.kill_switch.is_aborted() {
            for (index, point) in self.trail.iter().enumerate() {
                let age = elapsed(now, point.sampled_at);
                if age >= TRAIL_DECAY {
                    continue;
                }
                let fade = 1.0 - age.as_secs_f64() / TRAIL_DECAY.as_secs_f64();
                let head_weight = (index + 1) as f64 / self.trail.len() as f64;
                trail.push(raster::TrailSegment {
                    from: if index == 0 {
                        point.position
                    } else {
                        self.trail[index - 1].position
                    },
                    to: point.position,
                    alpha: fade * head_weight,
                    width: 3.0 + 7.0 * head_weight,
                });
            }
        }

        let ring = self.pre_click.map(|pulse| {
            let progress = (elapsed(now, pulse.started_at).as_secs_f64()
                / pulse.duration.as_secs_f64())
            .clamp(0.0, 1.0);
            raster::PreClickRing {
                center: pulse.center,
                progress,
                alpha: (1.0 - progress * 0.65).clamp(0.0, 1.0),
            }
        });
        let squash = self.click_squash.map(|click| {
            1.0 - 0.18 * (1.0 - eased_progress(now, click.started_at, click.duration))
        });
        OverlayScene {
            cursor: self.position,
            cursor_size: self.transform.cursor_size().unwrap_or(BASE_CURSOR_DIP),
            squash,
            trail,
            ring,
        }
    }
}

fn eased_progress(now: Duration, started_at: Duration, duration: Duration) -> f64 {
    if duration.is_zero() {
        return 1.0;
    }
    let linear = (elapsed(now, started_at).as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);
    1.0 - (1.0 - linear).powi(3)
}

fn elapsed(now: Duration, earlier: Duration) -> Duration {
    now.saturating_sub(earlier)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform(scale: f64, origin: ScreenPoint) -> CoordTransform {
        CoordTransform::new(
            scale,
            MonitorBounds::new(origin, 1920.0 * scale, 1080.0 * scale).unwrap(),
        )
        .unwrap()
    }

    fn state(reduced_motion: bool) -> OverlayState {
        OverlayState::new(
            ScreenPoint::new(0.0, 0.0),
            transform(1.0, ScreenPoint::new(0.0, 0.0)),
            reduced_motion,
            &KillSwitch::new(),
        )
        .unwrap()
    }

    #[test]
    fn coordinate_transform_scales_and_offsets_mixed_dpi() {
        let coord = transform(1.5, ScreenPoint::new(-1920.0, 100.0));
        let screen = coord
            .screenshot_to_screen(ScreenPoint::new(100.0, 200.0))
            .unwrap();
        assert_eq!(screen, ScreenPoint::new(-1770.0, 400.0));
        let screenshot = coord.screen_to_screenshot(screen).unwrap();
        assert_eq!(screenshot, ScreenPoint::new(100.0, 200.0));
        assert_eq!(coord.cursor_size().unwrap(), 72.0);
    }

    #[test]
    fn coordinate_transform_rejects_untrusted_numbers_and_clamps() {
        let coord = transform(1.25, ScreenPoint::new(300.0, -200.0));
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(coord
                .screenshot_to_screen(ScreenPoint::new(bad, 0.0))
                .is_err());
        }
        assert_eq!(
            coord
                .screenshot_to_screen(ScreenPoint::new(-500.0, 5000.0))
                .unwrap(),
            ScreenPoint::new(300.0, 1150.0)
        );
        assert!(CoordTransform::new(0.0, coord.monitor).is_err());
        assert!(CoordTransform::new(f64::MIN_POSITIVE, coord.monitor).is_err());
        assert!(MonitorBounds::new(ScreenPoint::new(f64::MAX, 0.0), f64::MAX, 1.0).is_err());
    }

    #[test]
    fn motion_never_teleports_and_lands_exactly() {
        assert_eq!(MOVE_DURATION, Duration::from_millis(300));
        let mut overlay = state(false);
        overlay
            .move_to(ScreenPoint::new(1000.0, 500.0), Duration::ZERO)
            .unwrap();
        assert!(overlay.needs_frame(Duration::ZERO));
        overlay.advance(Duration::from_millis(75));
        assert!(overlay.position.x > 0.0);
        assert!(overlay.position.x < 1000.0);
        overlay.advance(Duration::from_millis(150));
        assert!(overlay.position.x > 578.0);
        overlay.advance(MOVE_DURATION);
        assert_eq!(overlay.position, ScreenPoint::new(1000.0, 500.0));
        overlay.mark_frame_presented(MOVE_DURATION);
        overlay.advance(MOVE_DURATION + TRAIL_DECAY + Duration::from_millis(1));
        overlay.mark_frame_presented(MOVE_DURATION + TRAIL_DECAY + Duration::from_millis(1));
        assert!(!overlay.needs_frame(MOVE_DURATION + TRAIL_DECAY + Duration::from_millis(1)));
    }

    #[test]
    fn preclick_ring_and_click_squash_have_bounded_durations() {
        assert_eq!(PRECLICK_DURATION, Duration::from_millis(200));
        assert_eq!(CLICK_DURATION, Duration::from_millis(90));
        let mut overlay = state(false);
        overlay
            .move_to(ScreenPoint::new(300.0, 200.0), Duration::ZERO)
            .unwrap();
        overlay
            .command(OverlayCommand::PreClickPulse, Duration::ZERO)
            .unwrap();
        let ring = overlay.scene(Duration::from_millis(100)).ring.unwrap();
        assert_eq!(ring.center, ScreenPoint::new(300.0, 200.0));
        overlay.advance(PRECLICK_DURATION);
        assert!(overlay.scene(PRECLICK_DURATION).ring.is_none());
        overlay
            .command(OverlayCommand::Click, PRECLICK_DURATION)
            .unwrap();
        assert!(overlay.scene(PRECLICK_DURATION).squash.unwrap() < 1.0);
        overlay.advance(PRECLICK_DURATION + CLICK_DURATION);
        assert_eq!(
            overlay.scene(PRECLICK_DURATION + CLICK_DURATION).squash,
            None
        );
    }

    #[test]
    fn trail_is_capped_faded_and_disabled_by_reduced_motion() {
        assert_eq!(TRAIL_CAPACITY, 16);
        assert_eq!(TRAIL_DECAY, Duration::from_millis(180));
        let mut overlay = state(false);
        for index in 0..40 {
            let now = Duration::from_millis(index * 10);
            overlay
                .move_to(ScreenPoint::new(index as f64 * 20.0, 10.0), now)
                .unwrap();
            overlay.advance(now + Duration::from_millis(10));
        }
        assert_eq!(overlay.trail.len(), TRAIL_CAPACITY);
        let early = overlay.scene(Duration::from_millis(400));
        let late = overlay.scene(Duration::from_millis(490));
        assert!(!early.trail.is_empty());
        assert!(early.trail.iter().all(|segment| segment.alpha > 0.0));
        assert!(late.trail.iter().all(|segment| segment.alpha < 1.0));
        assert!(late.trail.last().unwrap().width > late.trail.first().unwrap().width);
        overlay.advance(Duration::from_millis(700));
        overlay.advance(Duration::from_millis(700) + TRAIL_IDLE_HIDE + TRAIL_DECAY);
        assert!(overlay.trail.is_empty());

        let mut calm = state(true);
        calm.move_to(ScreenPoint::new(200.0, 0.0), Duration::ZERO)
            .unwrap();
        calm.advance(Duration::from_millis(16));
        assert!(calm.trail.is_empty());
        assert!(calm.scene(Duration::from_millis(16)).trail.is_empty());
    }

    #[test]
    fn trail_hides_after_idle_and_frames_are_capped() {
        assert_eq!(TRAIL_IDLE_HIDE, Duration::from_millis(1000));
        assert_eq!(MAX_FRAME_RATE, 60);
        let mut overlay = state(false);
        assert!(overlay.needs_frame(Duration::ZERO));
        overlay.mark_frame_presented(Duration::ZERO);
        assert!(!overlay.needs_frame(Duration::from_millis(8)));
        overlay
            .move_to(ScreenPoint::new(100.0, 0.0), Duration::ZERO)
            .unwrap();
        assert!(!overlay.needs_frame(Duration::from_millis(8)));
        assert!(overlay.needs_frame(Duration::from_millis(17)));
        overlay.advance(MOVE_DURATION);
        overlay.advance(MOVE_DURATION + TRAIL_IDLE_HIDE + TRAIL_DECAY);
        overlay.mark_frame_presented(MOVE_DURATION + TRAIL_IDLE_HIDE + TRAIL_DECAY);
        assert!(!overlay.needs_frame(MOVE_DURATION + TRAIL_IDLE_HIDE + TRAIL_DECAY));
    }

    #[test]
    fn external_kill_switch_immediately_stops_and_hides() {
        let kill_switch = KillSwitch::new();
        let mut overlay = OverlayState::new(
            ScreenPoint::new(0.0, 0.0),
            transform(1.0, ScreenPoint::new(0.0, 0.0)),
            false,
            &kill_switch,
        )
        .unwrap();
        overlay
            .move_to(ScreenPoint::new(100.0, 0.0), Duration::ZERO)
            .unwrap();
        kill_switch.abort();
        assert!(!overlay.needs_frame(Duration::from_millis(100)));
        overlay.advance(Duration::from_millis(100));
        assert!(!overlay.visible);
        assert_eq!(overlay.position, ScreenPoint::new(0.0, 0.0));
        assert!(overlay
            .command(
                OverlayCommand::MoveTo(ScreenPoint::new(200.0, 0.0)),
                Duration::ZERO
            )
            .is_err());
    }

    #[test]
    fn moving_to_the_current_point_reshows_a_hidden_overlay() {
        let mut overlay = state(false);
        overlay
            .command(OverlayCommand::Hide, Duration::ZERO)
            .unwrap();
        assert!(!overlay.needs_frame(Duration::ZERO));
        overlay
            .move_to(ScreenPoint::new(0.0, 0.0), Duration::ZERO)
            .unwrap();
        assert!(overlay.visible);
        assert!(overlay.needs_frame(Duration::ZERO));
    }

    #[test]
    fn abort_command_trips_the_shared_kill_switch() {
        let kill_switch = KillSwitch::new();
        let mut overlay = OverlayState::new(
            ScreenPoint::new(0.0, 0.0),
            transform(1.0, ScreenPoint::new(0.0, 0.0)),
            false,
            &kill_switch,
        )
        .unwrap();
        overlay
            .command(OverlayCommand::Abort, Duration::ZERO)
            .unwrap();
        assert!(kill_switch.is_aborted());
        assert!(!overlay.visible);
        assert!(!overlay.needs_frame(Duration::ZERO));
    }

    #[test]
    fn action_pill_is_bounded_anchored_and_typing_parks_motion() {
        let mut overlay = state(false);
        overlay
            .move_to(ScreenPoint::new(100.0, 0.0), Duration::ZERO)
            .unwrap();
        overlay.advance(MOVE_DURATION);
        overlay.park_for_typing();
        assert!(overlay.motion.is_none());
        assert_eq!(overlay.position, ScreenPoint::new(100.0, 0.0));
        overlay
            .command(OverlayCommand::ParkForTyping, Duration::ZERO)
            .unwrap();
        overlay
            .command(
                OverlayCommand::SetActionPill("Click Save".to_string()),
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(overlay.action_pill(), Some("Click Save"));
        let anchor = overlay.action_pill_anchor().unwrap();
        assert!((anchor.x - 100.0).abs() < f64::EPSILON);
        assert!((anchor.y - (-64.8)).abs() < 0.000_001);
        assert!(overlay
            .command(
                OverlayCommand::SetActionPill("x".repeat(MAX_ACTION_PILL_BYTES + 1)),
                Duration::ZERO
            )
            .is_err());
        assert!(overlay.set_action_pill(&"x".repeat(49)).is_err());
        overlay.set_action_pill("").unwrap();
        assert_eq!(overlay.action_pill(), None);
        overlay
            .command(OverlayCommand::Hide, Duration::ZERO)
            .unwrap();
        assert!(!overlay.needs_frame(Duration::from_millis(16)));
    }
}
