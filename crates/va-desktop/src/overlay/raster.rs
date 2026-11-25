use crate::overlay::model::{ScreenPoint, BRAND_YELLOW, CURSOR_OUTLINE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pixel {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

impl Pixel {
    pub const TRANSPARENT: Self = Self {
        b: 0,
        g: 0,
        r: 0,
        a: 0,
    };

    pub const fn brand(alpha: u8) -> Self {
        Self::from_rgb(BRAND_YELLOW.0, BRAND_YELLOW.1, BRAND_YELLOW.2, alpha)
    }

    pub fn outline(alpha: u8) -> Self {
        Self::from_rgb(CURSOR_OUTLINE.0, CURSOR_OUTLINE.1, CURSOR_OUTLINE.2, alpha)
    }

    const fn from_rgb(r: u8, g: u8, b: u8, alpha: u8) -> Self {
        Self {
            b: ((b as u16 * alpha as u16 + 127) / 255) as u8,
            g: ((g as u16 * alpha as u16 + 127) / 255) as u8,
            r: ((r as u16 * alpha as u16 + 127) / 255) as u8,
            a: alpha,
        }
    }

    fn with_alpha(self, alpha: u8) -> Self {
        if self.a == 0 || alpha == 0 {
            return Self::TRANSPARENT;
        }
        let scale = |channel: u8| {
            ((u16::from(channel) * u16::from(alpha) + u16::from(self.a) / 2) / u16::from(self.a))
                .min(255) as u8
        };
        Self {
            b: scale(self.b),
            g: scale(self.g),
            r: scale(self.r),
            a: alpha,
        }
    }

    fn over(self, source: Self) -> Self {
        if source.a == 0 {
            return self;
        }
        if self.a == 0 {
            return source;
        }
        let inverse = 255 - u16::from(source.a);
        let channel = |dst: u8, src: u8| {
            (u16::from(src) + (u16::from(dst) * inverse + 127) / 255).min(255) as u8
        };
        Self {
            b: channel(self.b, source.b),
            g: channel(self.g, source.g),
            r: channel(self.r, source.r),
            a: (u16::from(source.a) + (u16::from(self.a) * inverse + 127) / 255).min(255) as u8,
        }
    }
}

#[derive(Debug)]
pub struct BgraBuffer {
    pub width: usize,
    pub height: usize,
    pixels: Vec<Pixel>,
}

impl BgraBuffer {
    pub fn new(width: usize, height: usize) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("overlay dimensions must be positive".to_string());
        }
        let len = width
            .checked_mul(height)
            .ok_or_else(|| "overlay dimensions overflow".to_string())?;
        Ok(Self {
            width,
            height,
            pixels: vec![Pixel::TRANSPARENT; len],
        })
    }

    #[cfg(test)]
    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    pub fn copy_bgra_to(&self, output: &mut [u8]) -> Result<(), String> {
        let expected = self
            .pixels
            .len()
            .checked_mul(4)
            .ok_or_else(|| "overlay byte length overflow".to_string())?;
        if output.len() != expected {
            return Err("BGRA output length does not match the overlay".to_string());
        }
        for (index, pixel) in self.pixels.iter().enumerate() {
            let offset = index * 4;
            output[offset..offset + 4].copy_from_slice(&[pixel.b, pixel.g, pixel.r, pixel.a]);
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.pixels.fill(Pixel::TRANSPARENT);
    }

    pub fn blend(&mut self, x: i64, y: i64, source: Pixel) {
        if source.a == 0 {
            return;
        }
        let Ok(x) = usize::try_from(x) else {
            return;
        };
        let Ok(y) = usize::try_from(y) else {
            return;
        };
        let Some(row) = y.checked_mul(self.width) else {
            return;
        };
        let Some(index) = x.checked_add(row) else {
            return;
        };
        if let Some(destination) = self.pixels.get_mut(index) {
            *destination = destination.over(source);
        }
    }

    fn stroke_segment(&mut self, from: ScreenPoint, to: ScreenPoint, width: f64, color: Pixel) {
        if !width.is_finite() || width <= 0.0 || color.a == 0 {
            return;
        }
        let radius = width * 0.5;
        let min_x = (from.x.min(to.x) - radius).floor().max(0.0) as i64;
        let max_x = (from.x.max(to.x) + radius)
            .ceil()
            .min(self.width as f64 - 1.0) as i64;
        let min_y = (from.y.min(to.y) - radius).floor().max(0.0) as i64;
        let max_y = (from.y.max(to.y) + radius)
            .ceil()
            .min(self.height as f64 - 1.0) as i64;
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let point = ScreenPoint::new(x as f64 + 0.5, y as f64 + 0.5);
                let distance = distance_to_segment(point, from, to);
                if distance > radius {
                    continue;
                }
                let coverage = ((radius - distance + 0.5) / 1.0).clamp(0.0, 1.0);
                let alpha = (f64::from(color.a) * coverage).round() as u8;
                self.blend(x, y, color.with_alpha(alpha));
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct TrailSegment {
    pub from: ScreenPoint,
    pub to: ScreenPoint,
    pub alpha: f64,
    pub width: f64,
}

#[derive(Clone, Debug)]
pub struct PreClickRing {
    pub center: ScreenPoint,
    pub progress: f64,
    pub alpha: f64,
}

#[derive(Clone, Debug, Default)]
pub struct OverlayScene {
    pub cursor: ScreenPoint,
    pub cursor_size: f64,
    pub squash: Option<f64>,
    pub trail: Vec<TrailSegment>,
    pub ring: Option<PreClickRing>,
}

pub fn render_scene(
    buffer: &mut BgraBuffer,
    scene: &OverlayScene,
    origin: ScreenPoint,
) -> Result<(), String> {
    if !finite_point(scene.cursor) || !finite_point(origin) {
        return Err("cursor and overlay origin must be finite".to_string());
    }
    if !scene.cursor_size.is_finite() || scene.cursor_size <= 0.0 {
        return Err("cursor size must be finite and positive".to_string());
    }
    for segment in &scene.trail {
        if !finite_point(segment.from)
            || !finite_point(segment.to)
            || !segment.alpha.is_finite()
            || !segment.width.is_finite()
        {
            return Err("trail values must be finite".to_string());
        }
    }
    if let Some(ring) = &scene.ring {
        if !finite_point(ring.center) || !ring.progress.is_finite() || !ring.alpha.is_finite() {
            return Err("ring values must be finite".to_string());
        }
    }
    if scene
        .squash
        .is_some_and(|squash| !squash.is_finite() || squash <= 0.0)
    {
        return Err("cursor squash must be finite and positive".to_string());
    }

    buffer.clear();
    render_screen_glow(buffer);
    for segment in &scene.trail {
        let alpha = (segment.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        if alpha == 0 || segment.width <= 0.0 {
            continue;
        }
        buffer.stroke_segment(
            segment.from - origin,
            segment.to - origin,
            segment.width,
            Pixel::brand(alpha),
        );
    }
    if let Some(ring) = &scene.ring {
        let progress = ring.progress.clamp(0.0, 1.0);
        let radius = scene.cursor_size * (0.38 + 0.32 * progress);
        let center = ring.center - origin;
        let alpha = (ring.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        let thickness = (scene.cursor_size * 0.055).max(1.5);
        let steps = 96;
        for step in 0..steps {
            let a = std::f64::consts::TAU * step as f64 / steps as f64;
            let b = std::f64::consts::TAU * (step + 1) as f64 / steps as f64;
            let from = center + ScreenPoint::new(a.cos() * radius, a.sin() * radius);
            let to = center + ScreenPoint::new(b.cos() * radius, b.sin() * radius);
            buffer.stroke_segment(
                from,
                to,
                thickness * 3.0,
                Pixel::brand((f64::from(alpha) * 0.18) as u8),
            );
            buffer.stroke_segment(from, to, thickness, Pixel::brand(alpha));
        }
    }
    render_cursor(
        buffer,
        scene.cursor - origin,
        scene.cursor_size,
        scene.squash,
    )
}

fn render_screen_glow(buffer: &mut BgraBuffer) {
    let band = (buffer.width.min(buffer.height) as f64 * 0.08).clamp(24.0, 96.0) as usize;
    let band = band.min(buffer.width).min(buffer.height);
    let alpha_at = |edge: usize| {
        let falloff = 1.0 - edge as f64 / band as f64;
        (falloff * falloff * 30.0).round() as u8
    };
    for y in 0..band {
        for x in 0..buffer.width {
            let edge = y.min(x).min(buffer.width - 1 - x);
            buffer.blend(x as i64, y as i64, Pixel::brand(alpha_at(edge)));
        }
    }
    for y in buffer.height - band..buffer.height {
        for x in 0..buffer.width {
            let edge = (buffer.height - 1 - y).min(x).min(buffer.width - 1 - x);
            buffer.blend(x as i64, y as i64, Pixel::brand(alpha_at(edge)));
        }
    }
    for y in band..buffer.height - band {
        for x in 0..band {
            let edge = x.min(y).min(buffer.height - 1 - y);
            buffer.blend(x as i64, y as i64, Pixel::brand(alpha_at(edge)));
        }
        for x in buffer.width - band..buffer.width {
            let edge = (buffer.width - 1 - x).min(y).min(buffer.height - 1 - y);
            buffer.blend(x as i64, y as i64, Pixel::brand(alpha_at(edge)));
        }
    }
}

fn render_cursor(
    buffer: &mut BgraBuffer,
    tip: ScreenPoint,
    size: f64,
    squash: Option<f64>,
) -> Result<(), String> {
    let squash = squash.unwrap_or(1.0);
    if !squash.is_finite() || squash <= 0.0 {
        return Err("cursor squash must be finite and positive".to_string());
    }
    let polygon = cursor_polygon(tip, size, squash);
    render_cursor_glow(buffer, &polygon, size * squash);
    if squash < 0.99 {
        render_click_rays(buffer, tip, size * squash, 1.0 - squash);
    }
    let outline_width = size * 0.055;
    let min_x = polygon
        .iter()
        .fold(f64::INFINITY, |x, point| x.min(point.x) - outline_width)
        .floor()
        .max(0.0) as i64;
    let max_x = polygon
        .iter()
        .fold(f64::NEG_INFINITY, |x, point| x.max(point.x) + outline_width)
        .ceil()
        .min(buffer.width as f64 - 1.0) as i64;
    let min_y = polygon
        .iter()
        .fold(f64::INFINITY, |y, point| y.min(point.y) - outline_width)
        .floor()
        .max(0.0) as i64;
    let max_y = polygon
        .iter()
        .fold(f64::NEG_INFINITY, |y, point| y.max(point.y) + outline_width)
        .ceil()
        .min(buffer.height as f64 - 1.0) as i64;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let point = ScreenPoint::new(x as f64 + 0.5, y as f64 + 0.5);
            let Some((fill, outline)) = cursor_coverage(&polygon, point, outline_width) else {
                continue;
            };
            if outline {
                buffer.blend(x, y, Pixel::outline(255));
            } else if fill {
                buffer.blend(x, y, Pixel::brand(255));
            }
        }
    }
    Ok(())
}

fn cursor_polygon(tip: ScreenPoint, size: f64, squash: f64) -> Vec<ScreenPoint> {
    let mut points = Vec::with_capacity(49);
    points.push(ScreenPoint::new(13.4049, 17.3737));
    push_cubic(
        &mut points,
        ScreenPoint::new(13.4049, 17.3737),
        ScreenPoint::new(12.1169, 20.1754),
        ScreenPoint::new(7.94756, 19.4443),
        ScreenPoint::new(7.6896, 16.3716),
    );
    points.push(ScreenPoint::new(7.10914, 9.45749));
    push_cubic(
        &mut points,
        ScreenPoint::new(7.10914, 9.45749),
        ScreenPoint::new(6.89678, 6.92797),
        ScreenPoint::new(9.72627, 5.29437),
        ScreenPoint::new(11.8107, 6.74304),
    );
    points.push(ScreenPoint::new(17.5083, 10.7028));
    push_cubic(
        &mut points,
        ScreenPoint::new(17.5083, 10.7028),
        ScreenPoint::new(20.0404, 12.4626),
        ScreenPoint::new(18.5888, 16.4388),
        ScreenPoint::new(15.5185, 16.1534),
    );
    points.push(ScreenPoint::new(14.7348, 16.0805));
    push_cubic(
        &mut points,
        ScreenPoint::new(14.7348, 16.0805),
        ScreenPoint::new(14.3122, 16.0412),
        ScreenPoint::new(13.9109, 16.273),
        ScreenPoint::new(13.7336, 16.6585),
    );
    points.push(ScreenPoint::new(13.4049, 17.3737));
    let hotspot = ScreenPoint::new(7.10914, 9.45749);
    let scale = size * squash / 24.0;
    points
        .into_iter()
        .map(|point| {
            tip + ScreenPoint::new((point.x - hotspot.x) * scale, (point.y - hotspot.y) * scale)
        })
        .collect()
}

fn push_cubic(
    points: &mut Vec<ScreenPoint>,
    from: ScreenPoint,
    control_a: ScreenPoint,
    control_b: ScreenPoint,
    to: ScreenPoint,
) {
    let steps = 10;
    for step in 1..=steps {
        let t = step as f64 / steps as f64;
        let u = 1.0 - t;
        let point = ScreenPoint::new(
            u * u * u * from.x
                + 3.0 * u * u * t * control_a.x
                + 3.0 * u * t * t * control_b.x
                + t * t * t * to.x,
            u * u * u * from.y
                + 3.0 * u * u * t * control_a.y
                + 3.0 * u * t * t * control_b.y
                + t * t * t * to.y,
        );
        points.push(point);
    }
}

fn render_cursor_glow(buffer: &mut BgraBuffer, polygon: &[ScreenPoint], size: f64) {
    let center = polygon
        .iter()
        .fold(ScreenPoint::new(0.0, 0.0), |sum, point| {
            sum + ScreenPoint::new(
                point.x / polygon.len() as f64,
                point.y / polygon.len() as f64,
            )
        });
    for (radius, alpha) in [(size * 0.82, 14), (size * 0.62, 24), (size * 0.44, 38)] {
        for step in 0..64 {
            let a = std::f64::consts::TAU * step as f64 / 64.0;
            let b = std::f64::consts::TAU * (step + 1) as f64 / 64.0;
            buffer.stroke_segment(
                center + ScreenPoint::new(a.cos() * radius, a.sin() * radius),
                center + ScreenPoint::new(b.cos() * radius, b.sin() * radius),
                size * 0.16,
                Pixel::brand(alpha),
            );
        }
    }
}

fn render_click_rays(buffer: &mut BgraBuffer, tip: ScreenPoint, size: f64, strength: f64) {
    let alpha = (strength.clamp(0.0, 1.0) * 220.0) as u8;
    let center = tip + ScreenPoint::new(size * 0.36, size * 0.42);
    for angle in [
        -std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
        -std::f64::consts::FRAC_PI_4,
    ] {
        let direction = ScreenPoint::new(angle.cos(), angle.sin());
        buffer.stroke_segment(
            center + direction * (size * 0.72),
            center + direction * (size * 0.98),
            (size * 0.045).max(1.5),
            Pixel::brand(alpha),
        );
    }
}

fn cursor_coverage(
    polygon: &[ScreenPoint],
    point: ScreenPoint,
    outline_width: f64,
) -> Option<(bool, bool)> {
    if point_in_polygon(polygon, point) {
        return Some((true, false));
    }
    for index in 0..polygon.len() {
        let from = polygon[index];
        let to = polygon[(index + 1) % polygon.len()];
        if distance_to_segment(point, from, to) <= outline_width {
            return Some((false, true));
        }
    }
    None
}

fn point_in_polygon(polygon: &[ScreenPoint], point: ScreenPoint) -> bool {
    let mut inside = false;
    for index in 0..polygon.len() {
        let a = polygon[index];
        let b = polygon[(index + 1) % polygon.len()];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}

fn distance_to_segment(point: ScreenPoint, from: ScreenPoint, to: ScreenPoint) -> f64 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f64::EPSILON {
        return (point.x - from.x).hypot(point.y - from.y);
    }
    let amount = ((point.x - from.x) * dx + (point.y - from.y) * dy) / length_squared;
    let amount = amount.clamp(0.0, 1.0);
    (point.x - (from.x + dx * amount)).hypot(point.y - (from.y + dy * amount))
}

fn finite_point(point: ScreenPoint) -> bool {
    point.x.is_finite() && point.y.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::model::{
        CoordTransform, KillSwitch, MonitorBounds, OverlayState, ScreenPoint,
    };
    use std::time::Duration;

    fn assert_premultiplied(buffer: &BgraBuffer) {
        assert!(buffer
            .pixels()
            .iter()
            .all(|pixel| pixel.r <= pixel.a && pixel.g <= pixel.a && pixel.b <= pixel.a));
    }

    fn emitted(buffer: &BgraBuffer) -> usize {
        buffer.pixels().iter().filter(|pixel| pixel.a > 0).count()
    }

    #[test]
    fn cursor_and_ring_emit_premultiplied_bgra_pixels() {
        let mut buffer = BgraBuffer::new(160, 160).unwrap();
        let scene = OverlayScene {
            cursor: ScreenPoint::new(50.0, 40.0),
            cursor_size: 56.0,
            squash: None,
            trail: Vec::new(),
            ring: Some(PreClickRing {
                center: ScreenPoint::new(90.0, 90.0),
                progress: 0.5,
                alpha: 0.8,
            }),
        };
        render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).unwrap();
        assert!(emitted(&buffer) > 100);
        assert_premultiplied(&buffer);
        assert!(buffer.pixels()[40 * 160 + 50].a > 0);
    }

    #[test]
    fn ribbon_emits_and_fades_without_violating_alpha() {
        let mut buffer = BgraBuffer::new(160, 80).unwrap();
        let scene = OverlayScene {
            cursor: ScreenPoint::new(140.0, 40.0),
            cursor_size: 56.0,
            squash: None,
            trail: vec![TrailSegment {
                from: ScreenPoint::new(20.0, 40.0),
                to: ScreenPoint::new(120.0, 40.0),
                alpha: 0.6,
                width: 8.0,
            }],
            ring: None,
        };
        render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).unwrap();
        assert!(emitted(&buffer) > 500);
        assert_premultiplied(&buffer);
        let yellow = Pixel::brand(153);
        assert_eq!(
            yellow,
            Pixel {
                b: 0,
                g: 110,
                r: 147,
                a: 153
            }
        );
        assert_eq!(
            yellow.with_alpha(77),
            Pixel {
                b: 0,
                g: 55,
                r: 74,
                a: 77
            }
        );
    }

    #[test]
    fn rendering_a_new_frame_clears_old_trail_pixels() {
        let mut buffer = BgraBuffer::new(160, 80).unwrap();
        let mut scene = OverlayScene {
            cursor: ScreenPoint::new(20.0, 40.0),
            cursor_size: 56.0,
            squash: None,
            trail: vec![TrailSegment {
                from: ScreenPoint::new(10.0, 40.0),
                to: ScreenPoint::new(140.0, 40.0),
                alpha: 1.0,
                width: 10.0,
            }],
            ring: None,
        };
        render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).unwrap();
        assert!(buffer.pixels()[40 * 160 + 100].a > 0);
        scene.cursor = ScreenPoint::new(140.0, 40.0);
        scene.trail.clear();
        render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).unwrap();
        assert_eq!(buffer.pixels()[40 * 160 + 100].a, 0);
        assert_premultiplied(&buffer);
    }

    #[test]
    fn rasterizer_rejects_invalid_scene_values() {
        let mut buffer = BgraBuffer::new(16, 16).unwrap();
        let mut scene = OverlayScene::default();
        assert!(render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).is_err());
        scene.cursor = ScreenPoint::new(0.0, 0.0);
        scene.cursor_size = 56.0;
        scene.squash = Some(f64::NAN);
        assert!(render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).is_err());
        scene.squash = None;
        render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).unwrap();
        assert!(emitted(&buffer) > 0);
    }

    #[test]
    fn cursor_hotspot_is_exactly_at_tip() {
        let mut buffer = BgraBuffer::new(80, 80).unwrap();
        let scene = OverlayScene {
            cursor: ScreenPoint::new(0.0, 0.0),
            cursor_size: 56.0,
            squash: None,
            trail: Vec::new(),
            ring: None,
        };
        render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).unwrap();
        assert!(buffer.pixels()[0].a > 0);
        assert!(buffer.pixels()[40 * 80 + 40].a == 0);
    }

    #[test]
    fn cursor_and_screen_glow_emit_soft_yellow_pixels() {
        let mut buffer = BgraBuffer::new(200, 120).unwrap();
        let scene = OverlayScene {
            cursor: ScreenPoint::new(100.0, 60.0),
            cursor_size: 48.0,
            squash: Some(0.82),
            trail: Vec::new(),
            ring: None,
        };
        render_scene(&mut buffer, &scene, ScreenPoint::new(0.0, 0.0)).unwrap();
        assert_premultiplied(&buffer);
        assert!(buffer.pixels()[0].a > 0);
        assert!(buffer.pixels()[60 * 200 + 95].a > 0);
        assert!(buffer.pixels()[60 * 200 + 95].r > 0);
        assert!(buffer.pixels()[60 * 200 + 95].g > 0);
        assert_eq!(buffer.pixels()[60 * 200 + 95].b, 0);
    }

    #[test]
    fn state_scene_render_is_premultiplied() {
        let transform = CoordTransform::new(
            1.0,
            MonitorBounds::new(ScreenPoint::new(0.0, 0.0), 1920.0, 1080.0).unwrap(),
        )
        .unwrap();
        let mut state = OverlayState::new(
            ScreenPoint::new(100.0, 100.0),
            transform,
            false,
            &KillSwitch::new(),
        )
        .unwrap();
        state
            .move_to(ScreenPoint::new(200.0, 100.0), Duration::ZERO)
            .unwrap();
        state.advance(Duration::from_millis(100));
        let mut buffer = BgraBuffer::new(320, 200).unwrap();
        render_scene(
            &mut buffer,
            &state.scene(Duration::from_millis(100)),
            ScreenPoint::new(0.0, 0.0),
        )
        .unwrap();
        assert_premultiplied(&buffer);
        assert!(emitted(&buffer) > 100);
    }
}
