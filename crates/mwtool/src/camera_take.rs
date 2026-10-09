//! `mwtool camera-take`: the ICE camera takes in `CAMERAS.BNDL` (`mw_formats::camera_take`), one line each, or
//! one take's layout and its camera evaluated every `--step` seconds.

use anyhow::{Context, Result, bail};
use mw_bundle::{Bundle, ResourceId, ResourceType};
use mw_formats::camera_take::{
    Bindings, CHANNEL_NAMES, CameraTake, ELEMENTS, FADE_COLOUR_NAMES, Frame, SHAKE_NAMES, TakeSample, el,
};

pub fn run(bundle: &Bundle, take: Option<&str>, step: f32) -> Result<()> {
    let mut takes = Vec::new();
    for e in bundle.entries_of_type(ResourceType::CameraTake) {
        let t = CameraTake::parse(&bundle.load(e)?).with_context(|| format!("take {}", e.id))?;
        takes.push((e.id, t));
    }
    takes.sort_by_key(|(id, _)| id.0);
    let Some(q) = take else {
        println!("{} takes ({{intervals}}/{{keys}} per channel in use)", takes.len());
        for (id, t) in &takes {
            let used: Vec<String> = t
                .channels
                .iter()
                .enumerate()
                .filter(|(_, c)| c.intervals > 0)
                .map(|(i, c)| format!("{} {}/{}", CHANNEL_NAMES[i], c.intervals, c.keys))
                .collect();
            println!(
                "{id} {:>8} {:6.2}s group {:08x}  {:<28} {}",
                t.unique_id,
                t.length,
                t.group_key,
                t.name,
                used.join(", ")
            );
        }
        return Ok(());
    };
    let pick = |id: &ResourceId, t: &CameraTake| {
        t.name == q
            || q.parse::<u32>().is_ok_and(|u| u == t.unique_id)
            || ResourceId::parse(q).is_some_and(|r| r == *id || r.0 == u64::from(t.unique_id))
    };
    let Some((id, t)) = takes.iter().find(|(id, t)| pick(id, t)) else {
        bail!("no take {q} (by name, UniqueId, or resource id; `mwtool camera-take` lists them)")
    };
    detail(*id, t, step.max(0.01));
    Ok(())
}

fn detail(id: ResourceId, t: &CameraTake, step: f32) {
    println!("{id} UniqueId {} '{}' {:.3}s, group {:08x}/{:08x}", t.unique_id, t.name, t.length, t.group, t.group_key);
    for (c, ch) in t.channels.iter().enumerate().filter(|(_, c)| c.intervals > 0) {
        let edges: Vec<String> = (0..=ch.intervals as usize)
            .map(|m| {
                let b = if m == 0 {
                    0.0
                } else if m < ch.intervals as usize {
                    ch.boundaries[m - 1] as f32 / 65535.0
                } else {
                    1.0
                };
                format!("{:.2}", b * t.length)
            })
            .collect();
        let keys: Vec<usize> = (0..ch.intervals as usize).map(|i| ch.key_of_interval(i)).collect();
        println!(
            "  {:<12} {} intervals over {} keys, edges {} s, first keys {:?}",
            CHANNEL_NAMES[c],
            ch.intervals,
            ch.keys,
            edges.join(" "),
            keys
        );
        // Per-interval values that are not all default, and keyed values of the non-camera channels.
        for (e, d) in ELEMENTS.iter().enumerate().filter(|(_, d)| d.channel as usize == c) {
            let n = if d.per_interval { ch.intervals } else { ch.keys } as usize;
            let vals: Vec<f32> = (0..n).map(|i| t.value(e, i)).collect();
            if vals.iter().all(|&v| v == d.default) && c != 0 {
                continue;
            }
            if c == 0 && !d.per_interval && e <= el::LOOK_Z {
                continue; // printed below, evaluated
            }
            let txt: Vec<String> = vals.iter().map(|v| format!("{v:.3}")).collect();
            println!("      {:<34} {}", d.name, txt.join(" "));
        }
    }
    println!(
        "camera every {step} s; eye / look in their spaces (game axes: x, y up, z forward of a Player space), \
         world = with identity bindings"
    );
    println!(
        "{:>6} {:>6}  {:>26} {:<9} {:>26} {:<9} {:>6} {:>5} {:>6} {:>5}  extra",
        "t", "f", "eye", "space", "look", "space", "fov", "lens", "dutch", "blend"
    );
    let bindings = Bindings { subject: Frame::IDENTITY, avatars: &[Frame::IDENTITY; 4] };
    let mut s = 0.0f32;
    loop {
        let x = t.sample(s);
        let cam = x.camera(&bindings);
        let es = x.eye_space();
        let ls = x.look_space();
        let fmt3 = |v: [f32; 3]| format!("{:8.3} {:8.3} {:8.3}", v[0], v[1], v[2]);
        let sp = |s: mw_formats::camera_take::Space| format!("{}#{}", s.name(), s.avatar);
        println!(
            "{s:6.2} {:6.3}  {} {:<9} {} {:<9} {:6.2} {:5.1} {:6.1} {:5.2}  {}",
            x.fraction,
            fmt3(x.eye()),
            sp(es),
            fmt3(x.look()),
            sp(ls),
            cam.fov_degrees,
            cam.lens_mm,
            cam.dutch_degrees,
            cam.game_camera_blend,
            extras(&x, &cam)
        );
        if s >= t.length {
            break;
        }
        s = (s + step).min(t.length);
    }
}

fn extras(x: &TakeSample, cam: &mw_formats::camera_take::IceCamera) -> String {
    let v = &x.values;
    let mut out = Vec::new();
    if x.eye_space_end() != x.eye_space() {
        out.push(format!(
            "eye->{}#{} {:.2}",
            x.eye_space_end().name(),
            x.eye_space_end().avatar,
            v[el::EYE_SPACE_BLEND]
        ));
    }
    if x.look_space_end() != x.look_space() {
        out.push(format!(
            "look->{}#{} {:.2}",
            x.look_space_end().name(),
            x.look_space_end().avatar,
            v[el::LOOK_SPACE_BLEND]
        ));
    }
    for (e, label) in
        [(el::NOROLL, "noroll"), (el::LOCK_TO_RENDER_OBJECT, "lock"), (el::AVOID_STATIC_OBJECTS, "avoid-static")]
    {
        if x.int(e) != 0 {
            out.push(label.into());
        }
    }
    if x.int(el::AVOID_DYNAMIC_OBJECTS) != 0 {
        out.push("avoid-dynamic".into());
    }
    if v[el::MOTION_BLUR_VEHICLE] != 0.0 || v[el::MOTION_BLUR_WORLD] != 0.0 {
        out.push(format!("mblur {:.3}/{:.3}", v[el::MOTION_BLUR_VEHICLE], v[el::MOTION_BLUR_WORLD]));
    }
    if v[el::FAR_FOCUS] != 0.0 || v[el::NEAR_FOCUS] != 0.0 || v[el::BLUR_INTENSITY] != 0.0 {
        out.push(format!(
            "focus near {:.2} far {:.2} falloff {:.2} blur {:.2}",
            v[el::NEAR_FOCUS],
            v[el::FAR_FOCUS],
            v[el::BLUR_FALLOFF],
            v[el::BLUR_INTENSITY]
        ));
    }
    if v[el::SHAKE_AMPLITUDE] != 0.0 {
        let ty = SHAKE_NAMES.get(x.int(el::SHAKE_TYPE) as usize).copied().unwrap_or("?");
        out.push(format!("shake {ty} amp {:.2} freq {:.2}", v[el::SHAKE_AMPLITUDE], v[el::SHAKE_FREQUENCY]));
    }
    if x.time_scale() != 1.0 {
        out.push(format!("time {:.2}", x.time_scale()));
    }
    if v[el::LETTERBOX] != 0.0 {
        out.push(format!("letterbox {}", v[el::LETTERBOX]));
    }
    if v[el::FADE] != 0.0 {
        let c = FADE_COLOUR_NAMES.get(x.int(el::FADE_TO_COLOR) as usize).copied().unwrap_or("?");
        out.push(format!("fade {} {c}", v[el::FADE]));
    }
    if v[el::EXPOSURE_BIAS] != 0.0 {
        out.push(format!("exposure {:.2}", v[el::EXPOSURE_BIAS]));
    }
    if x.int(el::LENS_OVERRIDE) != 0 {
        out.push(format!("fisheye {:.2} {:.2} {:.2}", v[el::FISHEYE_WARP], v[el::FISHEYE_SCALE], v[el::FISHEYE_POWER]));
    }
    if x.int(el::BYSTANDER_ENABLE) != 0 {
        out.push(format!(
            "bystander {:.1} m fov {:.0}..{:.0}",
            v[el::BYSTANDER_PERCEIVED_DISTANCE],
            v[el::BYSTANDER_MIN_FOV],
            v[el::BYSTANDER_MAX_FOV]
        ));
    }
    if x.int(el::EVENT_TAG) != 0 {
        out.push(format!("tag {}", x.int(el::EVENT_TAG)));
    }
    if x.int(el::OVERLAY) != 0 {
        out.push(format!("overlay {}", x.int(el::OVERLAY)));
    }
    let a = cam.approximations;
    if a.lag || a.bungee || a.scene_space {
        out.push(format!("(not modelled: lag {} bungee {} scene {})", a.lag, a.bungee, a.scene_space));
    }
    let f = cam.frame.z;
    out.push(format!("fwd {:.2} {:.2} {:.2}", f[0], f[1], f[2]));
    out.join(", ")
}
