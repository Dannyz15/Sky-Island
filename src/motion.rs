//! Spring motion. Everything that moves in the notch (opening, hiding, the tab highlight, hover
//! glows, switches) is pulled towards its goal by a damped spring instead of running on a fixed
//! timer. A spring can be turned around at any moment and carries its speed with it, which is what
//! makes the motion feel fluid instead of mechanical.

use std::cell::RefCell;
use std::collections::HashMap;

/// One step of a damped spring pulling `x` towards `target`. `omega` is the stiffness in radians per
/// second (higher is faster) and `zeta` the damping: 1 is critical (no overshoot), below 1 it bounces.
/// Time is cut into small slices so a slow frame cannot make it unstable. It lands exactly on the
/// target once it is close and slow enough.
pub fn spring(x: &mut f64, v: &mut f64, target: f64, dt: f64, omega: f64, zeta: f64) {
    let dt = dt.clamp(0.0, 1.0 / 15.0);
    // Slices short enough for the spring's own speed too: a very stiff spring (the Animation setting at
    // None) steps in tiny slices, or its damping would overshoot and jitter.
    let slices = (dt / 0.004).ceil().max((dt * omega / 0.25).ceil()).max(1.0) as u32;
    let h = dt / slices as f64;
    for _ in 0..slices {
        let accel = omega * omega * (target - *x) - 2.0 * zeta * omega * *v;
        *v += accel * h;
        *x += *v * h;
    }
    if (target - *x).abs() < 1e-4 && v.abs() < 1e-3 {
        *x = target;
        *v = 0.0;
    }
}

/// The key of a rectangle (logical pixels), so the same button gets the same animation every frame.
pub fn rect_key(r: (f32, f32, f32, f32)) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for f in [r.0, r.1, r.2, r.3] {
        h ^= (f * 4.0).round() as i64 as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

struct Entry {
    x: f64,
    v: f64,
    target: f64,
}

/// Smoothed values for things that animate on their own, such as a hover highlight fading in and out
/// or a switch knob sliding across, each found by a key.
#[derive(Default)]
pub struct Glows {
    map: RefCell<HashMap<u64, Entry>>,
    dt: RefCell<f64>,
    busy: RefCell<bool>,
    /// The Animation setting: how much faster than normal (0 means normal), and whether the bounce is off.
    speed: RefCell<f64>,
    flat: RefCell<bool>,
}

impl Glows {
    /// Start a frame: `dt` seconds have passed since the last one.
    pub fn begin(&self, dt: f64) {
        *self.dt.borrow_mut() = dt;
        *self.busy.borrow_mut() = false;
    }

    /// Apply the Animation setting to everything this keeps: `speed` times as fast as normal, and without
    /// the bounce when `bounce` is false.
    pub fn set_motion(&self, speed: f64, bounce: bool) {
        *self.speed.borrow_mut() = speed;
        *self.flat.borrow_mut() = !bounce;
    }

    /// How much faster than normal the Animation setting runs things.
    fn speed(&self) -> f64 {
        let speed = *self.speed.borrow();
        if speed > 0.0 { speed } else { 1.0 }
    }

    /// Whether anything is still moving, so another frame is needed.
    pub fn busy(&self) -> bool {
        *self.busy.borrow()
    }

    /// A value that follows `target` with a spring. `omega` and `zeta` are as in [`spring`].
    pub fn follow(&self, key: u64, target: f64, omega: f64, zeta: f64) -> f64 {
        let dt = *self.dt.borrow();
        let (omega, zeta) = (omega * self.speed(), if *self.flat.borrow() { 1.0 } else { zeta });
        let mut map = self.map.borrow_mut();
        let e = map.entry(key).or_insert(Entry { x: target, v: 0.0, target });
        e.target = target;
        spring(&mut e.x, &mut e.v, target, dt, omega, zeta);
        if e.x != target {
            *self.busy.borrow_mut() = true;
        }
        e.x
    }

    /// How fast a followed value is moving right now (0 for one not seen yet).
    pub fn velocity(&self, key: u64) -> f64 {
        self.map.borrow().get(&key).map_or(0.0, |e| e.v)
    }

    /// A 0 to 1 value that fades towards 1 while `on` and back to 0 otherwise, with no overshoot.
    /// A value seen for the first time starts at 0, so it fades in.
    pub fn fade(&self, key: u64, on: bool) -> f32 {
        let dt = *self.dt.borrow() * self.speed();
        let target = if on { 1.0 } else { 0.0 };
        let mut map = self.map.borrow_mut();
        let e = map.entry(key).or_insert(Entry { x: 0.0, v: 0.0, target });
        e.target = target;
        let k = 1.0 - (-dt * 20.0).exp();
        e.x += (target - e.x) * k;
        if (target - e.x).abs() < 0.004 {
            e.x = target;
        } else {
            *self.busy.borrow_mut() = true;
        }
        e.x as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(target: f64, omega: f64, zeta: f64, secs: f64) -> (f64, f64) {
        let (mut x, mut v) = (0.0, 0.0);
        let mut peak = 0.0f64;
        for _ in 0..(secs * 120.0) as u32 {
            spring(&mut x, &mut v, target, 1.0 / 120.0, omega, zeta);
            peak = peak.max(x);
        }
        (x, peak)
    }

    #[test]
    fn a_critical_spring_arrives_without_overshoot() {
        let (x, peak) = run(1.0, 26.0, 1.0, 1.0);
        assert_eq!(x, 1.0, "lands exactly on the target");
        assert!(peak <= 1.0 + 1e-9, "no overshoot, peaked at {peak}");
    }

    #[test]
    fn a_bouncy_spring_overshoots_a_little_and_then_settles() {
        let (x, peak) = run(1.0, 24.0, 0.74, 1.5);
        assert_eq!(x, 1.0);
        assert!(peak > 1.01 && peak < 1.08, "a small bounce, peaked at {peak}");
    }

    #[test]
    fn a_spring_reaches_most_of_the_way_in_a_fraction_of_a_second() {
        let (mut x, mut v) = (0.0, 0.0);
        for _ in 0..24 {
            spring(&mut x, &mut v, 1.0, 1.0 / 120.0, 26.0, 1.0);
        }
        assert!(x > 0.6, "after 0.2 s it is at {x}");
    }

    #[test]
    fn a_very_stiff_spring_still_settles_without_jitter() {
        // The Animation setting at None multiplies the stiffness by 8.
        let (mut x, mut v) = (0.0, 0.0);
        let mut last = 0.0;
        for _ in 0..30 {
            spring(&mut x, &mut v, 1.0, 1.0 / 60.0, 34.0 * 8.0, 1.0);
            assert!(x >= last - 1e-9 && x <= 1.0 + 1e-9, "x went from {last} to {x}");
            last = x;
        }
        assert_eq!(x, 1.0);
    }

    #[test]
    fn a_long_frame_does_not_blow_the_spring_up() {
        let (mut x, mut v) = (0.0, 0.0);
        for _ in 0..40 {
            spring(&mut x, &mut v, 1.0, 5.0, 30.0, 0.6);
            assert!(x.is_finite() && x.abs() < 3.0, "x = {x}");
        }
    }

    #[test]
    fn a_spring_can_be_turned_around_without_a_jump() {
        let (mut x, mut v) = (0.0, 0.0);
        for _ in 0..12 {
            spring(&mut x, &mut v, 1.0, 1.0 / 120.0, 26.0, 1.0);
        }
        let before = x;
        spring(&mut x, &mut v, 0.0, 1.0 / 120.0, 26.0, 1.0);
        assert!((x - before).abs() < 0.05, "moved {} in one frame after turning", (x - before).abs());
        for _ in 0..120 {
            spring(&mut x, &mut v, 0.0, 1.0 / 120.0, 26.0, 1.0);
        }
        assert_eq!(x, 0.0, "and it settles on the new target");
    }

    #[test]
    fn a_fade_goes_up_and_down_and_reports_when_it_is_moving() {
        let g = Glows::default();
        g.begin(1.0 / 60.0);
        assert_eq!(g.fade(1, false), 0.0);
        assert!(!g.busy());
        let mut last = 0.0;
        for _ in 0..4 {
            g.begin(1.0 / 60.0);
            let now = g.fade(1, true);
            assert!(now > last && now < 1.0 && g.busy());
            last = now;
        }
        for _ in 0..60 {
            g.begin(1.0 / 60.0);
            g.fade(1, true);
        }
        g.begin(1.0 / 60.0);
        assert_eq!(g.fade(1, true), 1.0);
        assert!(!g.busy(), "settled");
        g.begin(1.0 / 60.0);
        assert!(g.fade(1, false) < 1.0, "fades back out");
    }

    #[test]
    fn the_animation_setting_speeds_up_what_glows_keep() {
        let run = |speed: f64| {
            let g = Glows::default();
            g.set_motion(speed, true);
            g.begin(1.0 / 60.0);
            g.follow(1, 0.0, 30.0, 0.8);
            let mut x = 0.0;
            for _ in 0..6 {
                g.begin(1.0 / 60.0);
                x = g.follow(1, 1.0, 30.0, 0.8);
            }
            x
        };
        assert!(run(0.6) < run(1.0) && run(1.0) < run(1.7), "{} {} {}", run(0.6), run(1.0), run(1.7));
        let g = Glows::default();
        g.set_motion(8.0, false);
        g.begin(1.0 / 60.0);
        g.follow(1, 0.0, 30.0, 0.5);
        g.begin(1.0 / 60.0);
        assert!(g.follow(1, 1.0, 30.0, 0.5) > 0.9, "with no animation it lands almost at once");
    }

    #[test]
    fn followed_values_keep_their_own_state_per_key() {
        let g = Glows::default();
        g.begin(1.0 / 60.0);
        let a = g.follow(1, 1.0, 30.0, 0.7);
        let b = g.follow(2, 0.0, 30.0, 0.7);
        assert_eq!(a, 1.0, "first sight starts at the target");
        assert_eq!(b, 0.0);
        g.begin(1.0 / 60.0);
        let a2 = g.follow(1, 0.0, 30.0, 0.7);
        assert!(a2 < 1.0 && a2 > 0.0, "moving towards the new target");
        assert_eq!(g.follow(2, 0.0, 30.0, 0.7), 0.0, "the other key is untouched");
    }

    #[test]
    fn the_same_rectangle_gives_the_same_key() {
        assert_eq!(rect_key((1.0, 2.0, 3.0, 4.0)), rect_key((1.0, 2.0, 3.0, 4.0)));
        assert_ne!(rect_key((1.0, 2.0, 3.0, 4.0)), rect_key((1.0, 2.0, 3.0, 5.0)));
    }
}
