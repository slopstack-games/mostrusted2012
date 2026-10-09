//! `mwtool events`: the offline events (`mw_assets::events`), one line each, or one event in detail with its
//! route drawn over the traffic lane map.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, bail};
use mw_assets::AssetStore;
use mw_assets::events::{Event, EventKind, Events};

pub fn run(store: &AssetStore, id: Option<&str>, png: Option<&Path>, all: bool) -> Result<()> {
    let events = store.events()?;
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).ok();
    let text = |id: u32| -> String {
        if id == 0 {
            return "-".into();
        }
        lang.as_ref().and_then(|l| l.get(id)).map_or_else(|| format!("#{id}"), str::to_owned)
    };
    let cars: BTreeMap<u32, String> = lang
        .as_ref()
        .and_then(|l| store.vehicles(l).ok())
        .map(|v| v.into_iter().map(|c| (c.id, c.name)).collect())
        .unwrap_or_default();
    let car = |id: u32| cars.get(&id).cloned().unwrap_or_else(|| format!("#{id}"));

    let Some(id) = id else {
        summary(&events, &text, &car, all);
        return Ok(());
    };
    let id: u32 = id.parse()?;
    let Some(e) = events.get(id) else { bail!("no offline event {id} (mwtool events lists them)") };
    detail(&events, e, &text, &car);
    if let Some(png) = png {
        draw(store, e, png)?;
    }
    Ok(())
}

fn summary(events: &Events, text: &dyn Fn(u32) -> String, car: &dyn Fn(u32) -> String, all: bool) {
    let mut per_kind: BTreeMap<EventKind, usize> = BTreeMap::new();
    for e in &events.events {
        *per_kind.entry(e.kind).or_default() += 1;
    }
    println!("{} offline events:", events.events.len());
    for (k, n) in &per_kind {
        println!("  {:<14} {n}", k.label());
    }
    for (list, ids) in &events.event_lists {
        println!("EventList {list}: {} events", ids.len());
    }
    let in_list: std::collections::HashSet<u32> =
        events.event_lists.iter().flat_map(|(_, l)| l.iter().copied()).collect();
    println!("\n{:>8}  {:<12} {:>4} {:>3} {:>3}  {:<26} {:<24} type", "id", "kind", "laps", "cp", "ai", "name", "car");
    for e in &events.events {
        if !all && e.checkpoints.is_empty() && e.kind != EventKind::Ambush {
            continue;
        }
        let career = if in_list.contains(&e.id) { "" } else { " (not in an EventList)" };
        println!(
            "{:>8}  {:<12} {:>4} {:>3} {:>3}  {:<26} {:<24} {}{career}",
            e.id,
            e.kind.label(),
            e.laps,
            e.checkpoints.len(),
            e.opponents.len(),
            text(e.name),
            e.default_vehicle().map_or("-".into(), car),
            text(e.type_name),
        );
    }
    let missing: usize = events.events.iter().map(|e| e.missing_checkpoints.len()).sum();
    let no_spawn = events.events.iter().filter(|e| e.spawn.is_none()).count();
    let no_grid = events.events.iter().flat_map(|e| &e.opponents).filter(|o| o.start.is_none()).count();
    println!(
        "\nunresolved: {missing} checkpoint ids, {no_spawn} spawn positions, {no_grid} opponent placements \
         (none expected)"
    );
    if !all {
        println!("(events without checkpoints hidden; --all lists them)");
    }
}

fn detail(events: &Events, e: &Event, text: &dyn Fn(u32) -> String, car: &dyn Fn(u32) -> String) {
    println!("event {}: {} ({})", e.id, text(e.name), e.kind.label());
    println!("  description: {}", text(e.description));
    let mode = events.game_mode(e.game_mode);
    println!(
        "  game mode {} ({}), crash reset delay {} s; type {} ({})",
        e.game_mode,
        mode.map_or("-".into(), |m| text(m.name)),
        mode.map_or(f32::NAN, |m| m.crash_reset_delay),
        e.type_name,
        text(e.type_name)
    );
    println!(
        "  laps {}, nitrous {}, traffic {}, target speed {:?}, target time {:?}",
        e.laps, e.nitrous_allowed, e.traffic_enabled, e.target_speed, e.target_time
    );
    match &e.spawn {
        Some(s) => println!(
            "  start {} at ({:.1}, {:.1}, {:.1}) heading {:.1}°",
            s.id,
            s.position[0],
            s.position[1],
            s.position[2],
            s.direction[0].atan2(s.direction[2]).to_degrees()
        ),
        None => println!("  start {} (not a trigger point)", e.spawn_id),
    }
    println!("  cars:");
    for v in &e.allowed_vehicles {
        println!("    {:>8} {:<28} mods {:?} difficulty {}", v.vehicle, car(v.vehicle), v.mods, v.difficulty);
    }
    let route = e.route();
    let length: f32 = route.windows(2).map(|w| dist(w[0], w[1])).sum();
    println!(
        "  {} checkpoints, route {:.2} km per lap (straight lines between gates):",
        e.checkpoints.len(),
        length / 1000.0
    );
    for (i, g) in e.checkpoints.iter().enumerate() {
        let n = g.normal();
        println!(
            "    [{i:2}] {:>8} ({:8.1}, {:6.1}, {:8.1}) {:5.1} m wide, normal heading {:6.1}°",
            g.id,
            g.position[0],
            g.position[1],
            g.position[2],
            g.size[0],
            n[0].atan2(n[2]).to_degrees()
        );
    }
    if !e.missing_checkpoints.is_empty() {
        println!("  unresolved checkpoint ids: {:?}", e.missing_checkpoints);
    }
    println!("  {} opponents:", e.opponents.len());
    for o in &e.opponents {
        let at = o.start.map_or("unresolved".into(), |s| {
            format!("({:.1}, {:.1}, {:.1})", s.position[0], s.position[1], s.position[2])
        });
        println!(
            "    AI type {:>8}, placement {:>8} {at}, 9b4e35f7 {}, colour {}",
            o.ai_type, o.placement_id, o.unknown_9b4e, o.colour
        );
    }
    for a in &e.alternate_routes {
        println!("  alternate route at checkpoint {}: {:?}", a.checkpoint, a.alternatives);
    }
    println!(
        "  {} chevron lines ({} points)",
        e.chevrons.len(),
        e.chevrons.iter().map(|c| c.points.len()).sum::<usize>()
    );
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Top-down map around the route: lane network dim, the route yellow, gates white across the road, the
/// player's start green, opponents' starts red, chevron lines cyan. +z is down.
fn draw(store: &AssetStore, e: &Event, png: &Path) -> Result<()> {
    let route = e.route();
    if route.is_empty() {
        bail!("event {} has no start or checkpoints", e.id);
    }
    let margin = 250.0;
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in route.iter().chain(e.opponents.iter().filter_map(|o| o.start.as_ref()).map(|s| &s.position)) {
        (x0, z0, x1, z1) = (x0.min(p[0]), z0.min(p[2]), x1.max(p[0]), z1.max(p[2]));
    }
    (x0, z0, x1, z1) = (x0 - margin, z0 - margin, x1 + margin, z1 + margin);
    let scale = 2048.0 / (x1 - x0).max(z1 - z0);
    let (w, h) = (((x1 - x0) * scale) as u32 + 1, ((z1 - z0) * scale) as u32 + 1);
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([16, 16, 20]));
    let px = |x: f32, z: f32| (((x - x0) * scale) as i64, ((z - z0) * scale) as i64);
    let plot = |img: &mut image::RgbImage, x: f32, z: f32, c: [u8; 3], r: i64| {
        let (cx, cz) = px(x, z);
        for dz in -r..=r {
            for dx in -r..=r {
                let (a, b) = (cx + dx, cz + dz);
                if a >= 0 && b >= 0 && (a as u32) < w && (b as u32) < h {
                    img.put_pixel(a as u32, b as u32, image::Rgb(c));
                }
            }
        }
    };
    let line = |img: &mut image::RgbImage, a: [f32; 3], b: [f32; 3], c: [u8; 3], r: i64| {
        let steps = ((a[0] - b[0]).hypot(a[2] - b[2]) * scale) as usize + 1;
        for k in 0..=steps {
            let f = k as f32 / steps as f32;
            plot(img, a[0] + (b[0] - a[0]) * f, a[2] + (b[2] - a[2]) * f, c, r);
        }
    };
    if let Ok(t) = store.traffic_lanes() {
        for hull in &t.hulls {
            for s in &hull.sections {
                let rungs = &hull.rungs[s.first_rung as usize..][..s.rung_count as usize];
                for pair in rungs.windows(2) {
                    let mid = |r: &[[f32; 3]; 2]| [(r[0][0] + r[1][0]) * 0.5, 0.0, (r[0][2] + r[1][2]) * 0.5];
                    line(&mut img, mid(&pair[0]), mid(&pair[1]), [60, 60, 72], 0);
                }
            }
        }
    }
    for c in &e.chevrons {
        for pair in c.points.windows(2) {
            line(&mut img, pair[0], pair[1], [40, 200, 220], 1);
        }
    }
    for pair in route.windows(2) {
        line(&mut img, pair[0], pair[1], [240, 200, 40], 1);
    }
    if e.laps > 1
        && let (Some(last), Some(first)) = (e.checkpoints.last(), e.checkpoints.first())
    {
        line(&mut img, last.position, first.position, [240, 200, 40], 1);
    }
    for g in &e.checkpoints {
        let a = g.across();
        let half = g.size[0] * 0.5;
        let p = g.position;
        line(
            &mut img,
            [p[0] - a[0] * half, 0.0, p[2] - a[2] * half],
            [p[0] + a[0] * half, 0.0, p[2] + a[2] * half],
            [255, 255, 255],
            1,
        );
    }
    for o in &e.opponents {
        if let Some(s) = o.start {
            plot(&mut img, s.position[0], s.position[2], [230, 50, 50], 3);
        }
    }
    if let Some(s) = e.spawn {
        plot(&mut img, s.position[0], s.position[2], [40, 230, 60], 5);
        let tip = [s.position[0] + s.direction[0] * 40.0, 0.0, s.position[2] + s.direction[2] * 40.0];
        line(&mut img, s.position, tip, [40, 230, 60], 1);
    }
    img.save(png)?;
    println!("wrote {} ({w}x{h}, x {x0:.0}..{x1:.0}, z {z0:.0}..{z1:.0}, +z down)", png.display());
    Ok(())
}
