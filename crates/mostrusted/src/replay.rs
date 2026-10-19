//! `--replay ref/<name>/telemetry.csv`: drive the car with the inputs recorded from the original game
//! (`mwtool capture`) and log the same quantities, so the two can be compared run against run
//! (`mwtool compare`).
//!
//! The car first gets up to the recording's starting speed on the flat test track (steering straight), then the
//! recorded throttle, brake, steering and handbrake play back tick by tick, interpolated in time. Our telemetry
//! goes to `<recording>.ours.csv` with the column names of the recording where they mean the same thing.

use avian3d::prelude::{AngularVelocity, LinearVelocity};
use bevy::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};

const MPH: f32 = 0.44704;
/// Settle time before anything happens (s), as the handling tests.
const SETTLE: f32 = 1.5;
/// Give up getting to the start speed after this long (s).
const APPROACH_LIMIT: f32 = 60.0;

/// One recorded tick: time and the game's processed inputs.
#[derive(Debug, Clone, Copy)]
struct Row {
    t: f32,
    throttle: f32,
    brake: f32,
    steer: f32,
    handbrake: bool,
    /// The recorded nitrous lane (`in_nitrous`, vehicle +0x1c20), NaN without the column.
    nitrous: f32,
    /// The recorded speed (mph), for `MW_REPLAY_FOLLOW`.
    mph: f32,
    /// The recorded body position (x, z), for `MW_REPLAY_FOLLOW=path` (NaN without the columns).
    at: Vec2,
}

#[derive(Resource)]
pub struct Replay {
    rows: Vec<Row>,
    start_mph: f32,
    /// The recording's gear on its first row, engaged when the playback starts.
    start_gear: Option<i32>,
    /// Sign applied to the recorded steering: the game's axis may point the other way (ours: + = left). Found
    /// from the recording by correlating steering with yaw rate.
    steer_sign: f32,
    out_path: PathBuf,
    out: Option<std::io::BufWriter<std::fs::File>>,
    ticks: u32,
    /// Seconds into the playback (None while approaching the start speed).
    playing: Option<f32>,
    /// The last tick's playback time and inputs. This system runs before `car_physics`, so the car state it
    /// sees is the one the previous tick's inputs produced: each line pairs that state with those inputs, as a
    /// row of the recording does (logging this tick's inputs instead put ours one tick behind the original).
    prev: Option<(f32, [f32; 4])>,
    done: bool,
    /// Running in the world from `--spawn` (the recording's start point): the playback starts at once, the car
    /// given the recording's starting speed.
    pub world: bool,
}

impl Replay {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
        let col = |name: &str| header.iter().position(|h| *h == name);
        let need = |name: &str| col(name).ok_or_else(|| anyhow::anyhow!("{} has no column {name}", path.display()));
        let (ct, cth, cbr, cst) = (need("t")?, need("in_throttle")?, need("in_brake")?, need("in_steer")?);
        let (chb, cspeed, cgear) = (col("handbrake_a"), col("speed_mph"), col("gear"));
        let cnos = col("in_nitrous");
        let (cpx, cpz) = (col("body_px"), col("body_pz"));
        let yaw_cols = ["body_wx", "body_wy", "body_wz", "body_upx", "body_upy", "body_upz"].map(col);
        let mut rows = Vec::new();
        let mut start_mph = None;
        let mut start_gear = None;
        let mut corr = 0.0f64;
        for line in lines {
            let f: Vec<&str> = line.split(',').collect();
            let num = |c: usize| f.get(c).and_then(|v| v.parse::<f32>().ok());
            let (Some(t), Some(throttle), Some(brake), Some(steer)) = (num(ct), num(cth), num(cbr), num(cst)) else {
                continue;
            };
            let mph = cspeed.and_then(num).unwrap_or(0.0);
            start_mph.get_or_insert(mph);
            if let Some(g) = cgear.and_then(num) {
                start_gear.get_or_insert(g as i32);
            }
            if let [Some(a), Some(b), Some(c), Some(d), Some(e), Some(g)] = yaw_cols
                && let [Some(wx), Some(wy), Some(wz), Some(ux), Some(uy), Some(uz)] =
                    [num(a), num(b), num(c), num(d), num(e), num(g)]
                && mph.abs() > 10.0
            {
                corr += (steer * (wx * ux + wy * uy + wz * uz)) as f64;
            }
            let handbrake = chb.and_then(num).is_some_and(|h| h > 0.5 && h < 2.0);
            rows.push(Row {
                t,
                throttle: throttle.clamp(0.0, 1.0),
                brake: brake.clamp(0.0, 1.0),
                steer,
                handbrake,
                nitrous: cnos.and_then(num).unwrap_or(f32::NAN),
                mph,
                at: Vec2::new(cpx.and_then(num).unwrap_or(f32::NAN), cpz.and_then(num).unwrap_or(f32::NAN)),
            });
        }
        if rows.len() < 2 {
            anyhow::bail!("{}: not enough rows with inputs", path.display());
        }
        let t0 = rows[0].t;
        for r in &mut rows {
            r.t -= t0;
        }
        // Our steering is + = left with + yaw = left; if the game's input correlates negatively with its yaw
        // rate, its axis points the other way.
        let steer_sign = if corr < 0.0 { -1.0 } else { 1.0 };
        let mut out_path = path.to_path_buf();
        out_path.set_extension("ours.csv");
        info!(
            "replay: {} rows, {:.1} s, start {:.1} mph, steering sign {steer_sign} (yaw correlation {corr:.2})",
            rows.len(),
            rows.last().map_or(0.0, |r| r.t),
            start_mph.unwrap_or(0.0)
        );
        Ok(Self {
            rows,
            start_mph: start_mph.unwrap_or(0.0),
            start_gear,
            steer_sign,
            out_path,
            out: None,
            ticks: 0,
            playing: None,
            prev: None,
            done: false,
            world: false,
        })
    }

    /// `MW_REPLAY_FOLLOW=path`: steering towards the recorded path a short way ahead of the car's nearest point
    /// (pure pursuit; ours + = left), so a world replay drives the original's line.
    fn steer_to_path(&self, t: f32, pos: Vec2, forward: Vec2, speed: f32) -> Option<f32> {
        let i = self.rows.partition_point(|r| r.t <= t).min(self.rows.len() - 1);
        // The nearest recorded point within a few seconds of the playback time.
        let window = |r: &&Row| (r.t - t).abs() < 3.0 && r.at.is_finite();
        let near = self.rows.iter().filter(window).min_by(|a, b| a.at.distance(pos).total_cmp(&b.at.distance(pos)))?;
        let look = (speed.abs() * 0.6).max(4.0);
        let from = self.rows.iter().position(|r| std::ptr::eq(r, near)).unwrap_or(i);
        let mut acc = 0.0;
        let mut target = near.at;
        for w in self.rows[from..].windows(2) {
            acc += w[0].at.distance(w[1].at);
            target = w[1].at;
            if acc >= look {
                break;
            }
        }
        let d = (target - pos).normalize_or_zero();
        // Left of the heading is +: cross(forward, d) with y up is forward.y * d.x − forward.x * d.y in (x, z).
        let angle = (forward.y * d.x - forward.x * d.y).atan2(forward.dot(d));
        Some((angle * 3.0).clamp(-1.0, 1.0))
    }

    /// The recorded speed at playback time `t` (mph).
    fn mph_at(&self, t: f32) -> Option<f32> {
        let i = self.rows.partition_point(|r| r.t <= t);
        self.rows.get(i).map(|r| r.mph)
    }

    /// The recorded nitrous lane at playback time `t`, interpolated as the pedals (None without the column).
    fn nitrous_at(&self, t: f32) -> Option<f32> {
        let i = self.rows.partition_point(|r| r.t <= t);
        let b = self.rows.get(i)?;
        let a = if i == 0 { b } else { &self.rows[i - 1] };
        let k = if b.t > a.t { ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0) } else { 1.0 };
        let v = a.nitrous + (b.nitrous - a.nitrous) * k;
        (!v.is_nan()).then_some(v)
    }

    /// Inputs at playback time `t`, interpolated between recorded rows (the handbrake from the nearer row).
    fn inputs_at(&self, t: f32) -> Option<[f32; 4]> {
        let i = self.rows.partition_point(|r| r.t <= t);
        if i >= self.rows.len() {
            return None;
        }
        let b = self.rows[i];
        let a = if i == 0 { b } else { self.rows[i - 1] };
        let k = if b.t > a.t { ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0) } else { 1.0 };
        let lerp = |x: f32, y: f32| x + (y - x) * k;
        let hb = if k < 0.5 { a.handbrake } else { b.handbrake };
        Some([
            lerp(a.throttle, b.throttle),
            lerp(a.brake, b.brake),
            (lerp(a.steer, b.steer) * self.steer_sign).clamp(-1.0, 1.0),
            if hb { 1.0 } else { 0.0 },
        ])
    }
}

const COLUMNS: &str = "t,speed_mph,in_throttle,in_brake,in_steer,handbrake,rpm,gear,drifting,drift_scale,\
drift_rear_slip,yaw_rate,lat_accel,body_slip_deg,px,pz,w0_kappa,w0_alpha,w1_kappa,w1_alpha,w2_kappa,w2_alpha,\
w3_kappa,w3_alpha,wheel_angle,tyre_yaw_front,tyre_yaw_rear,drift_yaw_tq,drift_side_f,drift_hold_f,drift_yaw_scale,\
clutch,w0_omega,w1_omega,w2_omega,w3_omega,py,vy,pitch_rate,roll_rate,w0_cpy,w1_cpy,w2_cpy,w3_cpy,nos_mult";

/// Runs before `car_physics` in the fixed schedule: approach the start speed, then play the inputs back.
pub fn run_replay(
    time: Res<Time>,
    replay: Option<ResMut<Replay>>,
    mut input: ResMut<crate::drive::CarInput>,
    mut cars: Query<
        (&mut crate::drive::Car, &Transform, &mut LinearVelocity, &AngularVelocity),
        With<crate::drive::Player>,
    >,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut r), Ok((mut car, transform, mut velocity, angular))) = (replay, cars.single_mut()) else { return };
    if r.done {
        return;
    }
    let dt = time.delta_secs();
    r.ticks += 1;
    let now = r.ticks as f32 * dt;
    let mph = car.speed / MPH;
    let Some(t) = r.playing else {
        if now < SETTLE {
            input.scripted = Some([0.0; 4]);
            return;
        }
        // Approach: full throttle (or brake) until the start speed, steering straight.
        if r.world {
            // In the world the car starts where the recording does: give it the starting speed at once.
            velocity.0 = transform.rotation * Vec3::Z * r.start_mph * MPH;
        }
        if r.world || mph >= r.start_mph - 0.5 || now > SETTLE + APPROACH_LIMIT {
            info!("replay: starting playback at {mph:.1} mph");
            match std::fs::File::create(&r.out_path) {
                Ok(f) => {
                    let mut w = std::io::BufWriter::new(f);
                    let _ = writeln!(w, "{COLUMNS}");
                    r.out = Some(w);
                }
                Err(e) => warn!("replay: cannot write {}: {e}", r.out_path.display()),
            }
            r.playing = Some(0.0);
            if r.world {
                // Placed moving as the game's own placement does it (gear by speed, revs from the wheels).
                car.place_moving(r.start_mph * MPH);
            } else if let Some(gear) = r.start_gear.filter(|&g| g > 0) {
                car.drivetrain.put_in_gear(gear);
            }
        } else {
            // Full throttle: easing off near the target left fast cars short of it (part throttle selects the
            // cruising gearbox, which barely accelerates at 125 mph). One tick adds well under the 0.5 mph slack.
            input.scripted = Some([1.0, 0.0, 0.0, 0.0]);
        }
        return;
    };
    let Some(mut inputs) = r.inputs_at(t) else {
        if let Some(mut w) = r.out.take() {
            let _ = w.flush();
        }
        info!("replay: done, wrote {}", r.out_path.display());
        r.done = true;
        exit.write(AppExit::Success);
        return;
    };
    // `MW_REPLAY_FOLLOW=1`: throttle and brake hold the recorded speed instead of replaying the pedals (runs
    // over kerbs at the original's speed whatever the engines do; steering stays recorded).
    if std::env::var_os("MW_REPLAY_FOLLOW").is_some()
        && let Some(target) = r.mph_at(t)
    {
        let err = target - mph;
        inputs[0] = (err * 0.3).clamp(0.0, 1.0);
        inputs[1] = (-err * 0.3).clamp(0.0, 1.0);
        if std::env::var_os("MW_REPLAY_FOLLOW").is_some_and(|v| v == "path") {
            let p = transform.translation;
            let f = transform.rotation * Vec3::Z;
            if let Some(s) = r.steer_to_path(t, Vec2::new(p.x, p.z), Vec2::new(f.x, f.z).normalize_or_zero(), car.speed)
            {
                inputs[2] = s;
            }
        }
    }
    input.scripted = Some(inputs);
    input.scripted_nitrous = r.nitrous_at(t);
    let Some((t, inputs)) = r.prev.replace((t, inputs)) else {
        r.playing = Some(t + dt);
        return;
    };
    // The body's motion as the last physics step left it (the `Car` fields are read at the start of its tick).
    let (rot, v) = (transform.rotation, velocity.0);
    let (forward, up) = (rot * Vec3::Z, rot * Vec3::Y);
    let speed = v.dot(forward);
    // The game's speed (vehicle +0x1620): the speed over the ground signed by the direction of travel, not the
    // forward component (which reads low in a slide).
    let mph = v.length().copysign(speed) / MPH;
    let yaw_rate = angular.0.dot(up);
    let body_slip = v.dot(rot * Vec3::X).atan2(speed.abs().max(1.0));
    let slips: Vec<(f32, f32)> = car.wheel_slips().collect();
    let slip = |i: usize| slips.get(i).copied().unwrap_or_default();
    let spins: Vec<f32> = car.wheel_spins().collect();
    let spin = |i: usize| spins.get(i).copied().unwrap_or_default();
    let p = transform.translation;
    let local_w = rot.inverse() * angular.0;
    let contacts: Vec<f32> = car.wheel_contacts().map(|c| c.map_or(f32::NAN, |c| c.point.y)).collect();
    let cpy = |i: usize| contacts.get(i).copied().unwrap_or(f32::NAN);
    let line = format!(
        "{t:.4},{mph:.3},{:.3},{:.3},{:.3},{:.0},{:.0},{},{},{:.3},{:.2},{:.4},{:.3},{:.2},{:.2},{:.2},{:.3},{:.2},{:.3},{:.2},{:.3},{:.2},{:.3},{:.2},{:.5},{:.0},{:.0},{:.0},{:.0},{:.0},{:.4},{:.3},{:.2},{:.2},{:.2},{:.2},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}",
        inputs[0],
        inputs[1],
        inputs[2],
        inputs[3],
        car.rpm,
        car.gear,
        u8::from(car.drift.active()),
        car.drift.scale,
        car.rear_slip,
        yaw_rate,
        yaw_rate * speed,
        -body_slip.to_degrees(),
        p.x,
        p.z,
        slip(0).0,
        slip(0).1,
        slip(1).0,
        slip(1).1,
        slip(2).0,
        slip(2).1,
        slip(3).0,
        slip(3).1,
        car.steer_angle(),
        car.tyre_yaw[0],
        car.tyre_yaw[1],
        car.drift.last.yaw_torque,
        car.drift.last.side_force,
        car.drift.last.hold_force,
        car.drift.last.yaw_rate_scale,
        car.drivetrain.clutch,
        spin(0),
        spin(1),
        spin(2),
        spin(3),
        p.y,
        v.y,
        local_w.x,
        local_w.z,
        cpy(0),
        cpy(1),
        cpy(2),
        cpy(3),
    );
    if let Some(w) = &mut r.out {
        // The engine's nitrous multiplier the last tick used (`FUN_009d2aa0`, the original's vehicle +0x2a90).
        let _ = writeln!(w, "{line},{:.4}", car.drivetrain.nitrous_scale);
    }
    r.playing = r.prev.map(|(next, _)| next + dt);
}
