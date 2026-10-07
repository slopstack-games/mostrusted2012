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
}

#[derive(Resource)]
pub struct Replay {
    rows: Vec<Row>,
    start_mph: f32,
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
}

impl Replay {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
        let col = |name: &str| header.iter().position(|h| *h == name);
        let need = |name: &str| col(name).ok_or_else(|| anyhow::anyhow!("{} has no column {name}", path.display()));
        let (ct, cth, cbr, cst) = (need("t")?, need("in_throttle")?, need("in_brake")?, need("in_steer")?);
        let (chb, cspeed) = (col("handbrake_a"), col("speed_mph"));
        let yaw_cols = ["body_wx", "body_wy", "body_wz", "body_upx", "body_upy", "body_upz"].map(col);
        let mut rows = Vec::new();
        let mut start_mph = None;
        let mut corr = 0.0f64;
        for line in lines {
            let f: Vec<&str> = line.split(',').collect();
            let num = |c: usize| f.get(c).and_then(|v| v.parse::<f32>().ok());
            let (Some(t), Some(throttle), Some(brake), Some(steer)) = (num(ct), num(cth), num(cbr), num(cst)) else {
                continue;
            };
            let mph = cspeed.and_then(num).unwrap_or(0.0);
            start_mph.get_or_insert(mph);
            if let [Some(a), Some(b), Some(c), Some(d), Some(e), Some(g)] = yaw_cols
                && let [Some(wx), Some(wy), Some(wz), Some(ux), Some(uy), Some(uz)] =
                    [num(a), num(b), num(c), num(d), num(e), num(g)]
                && mph.abs() > 10.0
            {
                corr += (steer * (wx * ux + wy * uy + wz * uz)) as f64;
            }
            let handbrake = chb.and_then(num).is_some_and(|h| h > 0.5 && h < 2.0);
            rows.push(Row { t, throttle: throttle.clamp(0.0, 1.0), brake: brake.clamp(0.0, 1.0), steer, handbrake });
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
            steer_sign,
            out_path,
            out: None,
            ticks: 0,
            playing: None,
            prev: None,
            done: false,
        })
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
w3_kappa,w3_alpha,wheel_angle,tyre_yaw_front,tyre_yaw_rear,drift_yaw_tq,drift_side_f,drift_hold_f,drift_yaw_scale";

/// Runs before `car_physics` in the fixed schedule: approach the start speed, then play the inputs back.
pub fn run_replay(
    time: Res<Time>,
    replay: Option<ResMut<Replay>>,
    mut input: ResMut<crate::drive::CarInput>,
    cars: Query<(&crate::drive::Car, &Transform, &LinearVelocity, &AngularVelocity)>,
    mut exit: MessageWriter<AppExit>,
) {
    let (Some(mut r), Ok((car, transform, velocity, angular))) = (replay, cars.single()) else { return };
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
        if mph >= r.start_mph - 0.5 || now > SETTLE + APPROACH_LIMIT {
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
        } else {
            let throttle = ((r.start_mph - mph) / 5.0).clamp(0.2, 1.0);
            input.scripted = Some([throttle, 0.0, 0.0, 0.0]);
        }
        return;
    };
    let Some(inputs) = r.inputs_at(t) else {
        if let Some(mut w) = r.out.take() {
            let _ = w.flush();
        }
        info!("replay: done, wrote {}", r.out_path.display());
        r.done = true;
        exit.write(AppExit::Success);
        return;
    };
    input.scripted = Some(inputs);
    let Some((t, inputs)) = r.prev.replace((t, inputs)) else {
        r.playing = Some(t + dt);
        return;
    };
    // The body's motion as the last physics step left it (the `Car` fields are read at the start of its tick).
    let (rot, v) = (transform.rotation, velocity.0);
    let (forward, up) = (rot * Vec3::Z, rot * Vec3::Y);
    let speed = v.dot(forward);
    let mph = speed / MPH;
    let yaw_rate = angular.0.dot(up);
    let body_slip = v.dot(rot * Vec3::X).atan2(speed.abs().max(1.0));
    let slips: Vec<(f32, f32)> = car.wheel_slips().collect();
    let slip = |i: usize| slips.get(i).copied().unwrap_or_default();
    let p = transform.translation;
    let line = format!(
        "{t:.4},{mph:.3},{:.3},{:.3},{:.3},{:.0},{:.0},{},{},{:.3},{:.2},{:.4},{:.3},{:.2},{:.2},{:.2},{:.3},{:.2},{:.3},{:.2},{:.3},{:.2},{:.3},{:.2},{:.5},{:.0},{:.0},{:.0},{:.0},{:.0},{:.4}",
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
    );
    if let Some(w) = &mut r.out {
        let _ = writeln!(w, "{line}");
    }
    r.playing = r.prev.map(|(next, _)| next + dt);
}
