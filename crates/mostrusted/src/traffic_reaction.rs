//! A traffic car's reaction once a contact has made it physical (docs/decomp.md "Traffic reactions"):
//! the request type of the contact (`FUN_005f2eb0`), manoeuvre 1 (`FUN_005f5c40`: the horn wish, a parked
//! car's alarm wish, its 5 s end), the headlight flash it holds (`FUN_00ebe8c0` → `FUN_00614620`) and the light
//! state machine (`FUN_005f53b0`) whose brightness byte sets the car lights' intensity (`FUN_005fd190`).
//! Engine-free; `traffic.rs` drives it per crashed car, `car_sfx.rs` plays the horn and the alarm.

/// `.data` 0x010c4d24: mph → m/s.
const MPH: f32 = 0.447_04;
/// `FUN_005f2eb0`: request 4 needs the contact faster than 30 mph (`.rdata` 0x00fb1ec0) and the traffic car faster
/// than 40 mph (0x00fb1ec4), with the contact in box zone 5..=9 (`FUN_00929da0`: side or rear).
const HARD_CONTACT_MPH: f32 = 30.0;
const HARD_CAR_MPH: f32 = 40.0;
/// `FUN_005f5c40`: a lane car leaves manoeuvre 1 once its physical time (+0x200) passes 5 s (`.rdata`
/// 0x00fb278c).
const MANOEUVRE_1_TIME: f32 = 5.0;
/// `FUN_005f5c40`: the "standing" test, every velocity component within 0.5 m/s (`_DAT_00fb2230`).
const STANDING: f32 = 0.5;

/// Flash patterns (`.rdata` 0x00fb2168, three rows of nine step times in seconds; ≤ 0 ends the pattern). Even
/// steps are dark, odd steps lit (`FUN_005f53b0`).
pub const FLASH_PATTERNS: [[f32; 9]; 3] = [
    [0.15, 0.30, 0.13, 0.25, 0.14, 0.25, 0.18, -1.0, -1.0],
    [0.12, 0.20, 0.10, 0.20, 0.09, 0.16, 0.21, 0.20, 0.18],
    [0.16, 0.75, 0.22, 0.25, 0.16, 0.32, 0.13, -1.0, -1.0],
];

/// Car +0x60 bits used here.
const LIT: u16 = 0x08;
const FLASH: u16 = 0x100;

/// The car's light state (car +0x54 step time, +0x60 bits 0x08 / 0x100, +0x64 brightness, +0x66 pattern, +0x67
/// step). Not ported: the hazard blink (0x20) and the indicators (0x40 / 0x80, +0x65).
#[derive(Clone, Debug)]
pub struct CarLights {
    flags: u16,
    step_time: f32,
    pattern: u8,
    step: u8,
    /// +0x64; 0xff at the car's set-up (`FUN_005f50b0`), ramps to 0 while unlit.
    pub level: u8,
}

impl Default for CarLights {
    fn default() -> Self {
        Self { flags: 0, step_time: 0.0, pattern: 0, step: 0, level: 0xff }
    }
}

impl CarLights {
    /// `FUN_00614620(on)`: start a flash pattern (`draw` = the generator's high word, pattern = draw % 3) unless
    /// one runs; `FUN_00614620(off)` clears it.
    pub fn set_flash(&mut self, on: bool, draw: impl FnOnce() -> u32) {
        if (self.flags & FLASH != 0) == on {
            return;
        }
        if on {
            self.flags |= FLASH | LIT;
            self.step_time = 0.0;
            self.step = 0;
            self.pattern = (draw() % 3) as u8;
        } else {
            self.flags &= !(FLASH | LIT);
        }
    }

    #[cfg(test)]
    pub fn flashing(&self) -> bool {
        self.flags & FLASH != 0
    }

    /// `FUN_005f53b0` (flash part): step the pattern, then move the brightness by `fade` (`int(TrafficVehicleConfig
    /// +0x28 × dt)`) towards 255 (lit) or 0. A parked car's brightness stays 0 (no hazard blink here).
    pub fn update(&mut self, dt: f32, fade: i32, parked: bool) {
        if self.flags & FLASH != 0 {
            let row = &FLASH_PATTERNS[self.pattern as usize];
            let mut step = self.step as usize;
            let mut d = row[step];
            let mut t = self.step_time;
            if d > 0.0 && step < 8 {
                t += dt;
                if d <= t {
                    step += 1;
                    t -= d;
                    d = row[step];
                }
            }
            self.step = step as u8;
            self.step_time = t;
            let active = d > 0.0 && step <= 8;
            if active && step.is_multiple_of(2) {
                self.flags &= !LIT;
            } else {
                self.flags |= LIT;
                if !active {
                    self.flags &= !(FLASH | LIT);
                }
            }
        }
        let level =
            if self.flags & LIT != 0 { (self.level as i32 + fade).min(255) } else { (self.level as i32 - fade).max(0) };
        self.level = if parked { 0 } else { level as u8 };
    }

    /// `FUN_005fd190`: the car lights' intensity, `b / 255 × 0.5 + 0.7` (`.rdata` 0x00f91584, 0x00f8a810,
    /// 0x00faece8), 0 for a parked car or one in manoeuvre 10 (wrecked).
    pub fn intensity(&self, parked: bool, wrecked: bool) -> f32 {
        if parked || wrecked { 0.0 } else { self.level as f32 / 255.0 * 0.5 + 0.7 }
    }
}

/// `FUN_00929da0`: the zone (0..=10) of a contact point given in the car's frame relative to its box centre,
/// scaled by the half extents and then by the largest component.
pub fn impact_zone(local: [f32; 3], half: [f32; 3]) -> u8 {
    let q = [local[0] / half[0].max(1e-3), local[1] / half[1].max(1e-3), local[2] / half[2].max(1e-3)];
    let m = q.iter().fold(0.0f32, |a, v| a.max(v.abs()));
    if m <= 0.0 {
        return 10;
    }
    let (x, y, z) = (q[0] / m, q[1] / m, q[2] / m);
    let under = y < -0.8 && y.abs() > 0.8 + 0.4 * (x.abs() - 0.5);
    if x > 0.5 {
        if z > 0.7 {
            2
        } else if z < -0.7 {
            8
        } else if under {
            1
        } else {
            9
        }
    } else if x < -0.5 {
        if z > 0.7 {
            4
        } else if z < -0.7 {
            6
        } else if under {
            1
        } else {
            5
        }
    } else if z > 0.7 {
        3
    } else if z < -0.7 {
        7
    } else if y > 0.8 {
        0
    } else if under {
        1
    } else {
        10
    }
}

/// `FUN_005f2eb0`: the request a contact gives a lane car (4 for a hard side / rear hit on a fast car, else 6).
pub fn contact_request(contact_speed: f32, car_speed: f32, zone: u8) -> u8 {
    if contact_speed > HARD_CONTACT_MPH * MPH && car_speed > HARD_CAR_MPH * MPH && (5..=9).contains(&zone) {
        4
    } else {
        6
    }
}

/// A physical traffic car's reaction state.
#[derive(Clone, Debug)]
pub struct Reaction {
    /// The manoeuvre (+0x6f): `Some(1)` while in manoeuvre 1; `None` for any other (not ported).
    pub manoeuvre: Option<u8>,
    /// Physical time (+0x200, `FUN_005ffa10`).
    pub time: f32,
    /// The car's random value (+0x5c).
    pub random: f32,
    pub parked: bool,
    pub lights: CarLights,
    /// Horn wish (+0x60 bit 0).
    pub horn: bool,
    /// Alarm wish (+0x60 bit 0x10).
    pub alarm: bool,
}

impl Reaction {
    /// `FUN_00609b70` → `FUN_006071d0`: a parked car always asks for 2; requests 2 and 6 score manoeuvre 1
    /// (`FUN_005eac20`, no trailer); request 4 picks manoeuvre 6 / 2 / 4 (not ported).
    pub fn new(request: u8, parked: bool, random: f32) -> Self {
        let request = if parked { 2 } else { request };
        let manoeuvre = matches!(request, 2 | 6).then_some(1);
        Self { manoeuvre, time: 0.0, random, parked, lights: CarLights::default(), horn: false, alarm: false }
    }

    /// One update: manoeuvre 1 (`FUN_005f5c40`), the flash (`FUN_00ebe8c0`) and the lights (`FUN_005f53b0`).
    /// `velocity` is the car body's; `horn_chance` = `TrafficParamConfig` +0x20.
    pub fn update(&mut self, dt: f32, fade: i32, velocity: [f32; 3], horn_chance: f32, draw: impl FnOnce() -> u32) {
        self.time += dt;
        self.horn = false;
        if self.manoeuvre == Some(1) {
            // MODEL: the "moving" test is the standing test alone (the game also needs the direction term > 0).
            let moving = velocity.iter().any(|v| v.abs() > STANDING);
            if self.parked && moving {
                self.alarm = true;
            } else {
                self.horn = self.random <= horn_chance;
            }
            if !self.parked && self.time > MANOEUVRE_1_TIME {
                // Returns 0: manoeuvre 0 (or 10 once the damage bar is full).
                self.manoeuvre = None;
            }
        }
        self.lights.set_flash(self.manoeuvre == Some(1), draw);
        self.lights.update(dt, fade, self.parked);
    }

    /// The car lights' intensity relative to the normal 0.7 (1 when not flashing, up to 1.71).
    pub fn light_gain(&self) -> f32 {
        if self.parked { 1.0 } else { self.lights.intensity(false, false) / 0.7 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_0_flashes_three_times() {
        let mut l = CarLights::default();
        l.set_flash(true, || 0);
        let (dt, mut lit, mut was, mut t) = (1.0 / 120.0, 0, false, 0.0);
        while t < 2.0 {
            l.update(dt, 41, false);
            let on = l.flags & LIT != 0;
            if on && !was {
                lit += 1;
            }
            was = on;
            t += dt;
        }
        assert_eq!(lit, 3);
        assert!(!l.flashing());
        assert_eq!(l.level, 0);
    }

    #[test]
    fn zones() {
        let h = [1.0, 0.7, 2.3];
        assert_eq!(impact_zone([0.0, 0.0, 2.3], h), 3);
        assert_eq!(impact_zone([0.0, 0.0, -2.3], h), 7);
        assert_eq!(impact_zone([1.0, 0.0, 0.0], h), 9);
        assert_eq!(impact_zone([-1.0, 0.0, 0.0], h), 5);
        assert_eq!(contact_request(20.0, 20.0, 9), 4);
        assert_eq!(contact_request(20.0, 20.0, 3), 6);
    }

    #[test]
    fn manoeuvre_1_ends_after_5_s() {
        let mut r = Reaction::new(6, false, 0.3);
        for _ in 0..600 {
            r.update(1.0 / 100.0, 50, [0.0; 3], 0.4, || 1);
        }
        assert_eq!(r.manoeuvre, None);
        r = Reaction::new(6, false, 0.3);
        r.update(0.01, 50, [0.0; 3], 0.4, || 1);
        assert!(r.horn && r.lights.flashing());
    }
}
