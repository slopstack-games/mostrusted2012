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
        match store.route_markers() {
            Ok(m) => println!(
                "route markers (GameLogicConfig.CheckpointConfig): arrows {:?}\n  block {:?}\n  POIs {:?}",
                m.arrows, m.block, m.poi
            ),
            Err(e) => println!("route markers: {e}"),
        }
        return Ok(());
    };
    let id: u32 = id.parse()?;
    let Some(e) = events.get(id) else { bail!("no offline event {id} (mwtool events lists them)") };
    detail(&events, e, &text, &car);
    sequences(store, &events, e);
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
        let mut ai_cars: Vec<String> =
            e.opponents.iter().map(|o| o.rollout.map_or("?".into(), |r| car(r.vehicle))).collect();
        ai_cars.dedup();
        let ai_cars = if ai_cars.is_empty() { String::new() } else { format!("; AI: {}", ai_cars.join(", ")) };
        println!(
            "{:>8}  {:<12} {:>4} {:>3} {:>3}  {:<26} {:<24} {}{career}{ai_cars}",
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
        "  game mode {} ({}): intro {} s, timeout {} s, crash reset delay {} s, gate markers {}; type {} ({})",
        e.game_mode,
        mode.map_or("-".into(), |m| text(m.name)),
        mode.map_or(f32::NAN, |m| m.intro_time),
        mode.map_or(f32::NAN, |m| m.timeout),
        mode.map_or(f32::NAN, |m| m.crash_reset_delay),
        mode.is_some_and(|m| m.gate_markers),
        e.type_name,
        text(e.type_name)
    );
    println!(
        "  laps {}, nitrous {}, traffic {}, first place only {}, skip the player's car's AI {}, target speed {:?}, \
         target time {:?}, heat levels {:?}",
        e.laps,
        e.nitrous_allowed,
        e.traffic_enabled,
        e.first_place_only,
        e.skip_player_car_ai,
        e.target_speed,
        e.target_time,
        e.heat_levels
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
        let rollout = o.rollout.map_or("no rollout".into(), |r| {
            let up = [r.body_upgrade, r.nitrous_upgrade, r.tyre_upgrade];
            format!("rollout {} car {} ({}), colour {}, upgrades {up:?}", r.id, r.vehicle, car(r.vehicle), r.colour)
        });
        println!(
            "    AI type {:>8}{}{}, placement {:>8} {at}, colour {} (else {}); {rollout}",
            o.ai_type,
            if o.most_wanted { " (Most Wanted)" } else { "" },
            if o.cop { " (cop)" } else { "" },
            o.placement_id,
            o.colour,
            o.alt_colour
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

/// The event's intro and outro sequences, item by item (`mw_assets::sequence`).
fn sequences(store: &AssetStore, events: &Events, e: &Event) {
    use mw_assets::sequence::{Controller, ItemKind};
    let mode = events.game_mode(e.game_mode);
    println!(
        "  sequences: intro {} (group {}), outro {}, AR outro {}, feedback group {}; game mode: AR outro {}, \
         countdown {}, feedback group id {}",
        e.event_intro,
        e.intro_message_group,
        e.event_outro,
        e.ar_outro,
        e.feedback_message_group,
        mode.map_or(0, |m| m.ar_outro),
        mode.map_or(0, |m| m.countdown_sequence),
        mode.map_or(0, |m| m.feedback_group_id),
    );
    let (intro, outro) = match store.event_sequences(e, mode) {
        Ok(s) => s,
        Err(err) => {
            println!("  sequences: {err}");
            return;
        }
    };
    for (label, seq) in [("intro", intro), ("outro", outro)] {
        let Some(seq) = seq else {
            println!("  {label}: none");
            continue;
        };
        println!("  {label} {} ({:016x}): {:.2} s, {} items", seq.unique_id, seq.id.0, seq.length, seq.items.len());
        for c in &seq.controllers {
            match c {
                Controller::JumpOnButtonPress { trigger_time, destination, .. } => {
                    println!("    skip: from {trigger_time:.2} s a press jumps to {destination:.2} s")
                }
                Controller::Jump { kind, trigger_time, destination, .. } => {
                    println!("    {kind}: at {trigger_time:.2} s -> {destination:.2} s (not run)")
                }
            }
        }
        for i in &seq.items {
            let what = match &i.kind {
                ItemKind::AnimatedCamera(c) => {
                    let k = c.key_at(0.0);
                    format!(
                        "animated camera {:?} (skeleton {:?}), {} frames at {} fps, focal {:.1}..{:.1} mm, relative to {:?}, first key {:?}",
                        c.animation_id.map(|a| format!("{:016x}", a.0)),
                        c.skeleton_id.map(|a| format!("{:x}", a.0 & 0xffff_ffff)),
                        c.keys.len(),
                        c.frame_rate,
                        c.focal_length.at(0.0),
                        c.focal_length.at(1.0),
                        c.relative_to,
                        k.map(|k| k.translation)
                    )
                }
                ItemKind::Camera(shots) => shots
                    .iter()
                    .map(|s| match &s.bystander {
                        Some(b) => format!("bystander {}", b.camera_binding),
                        None => format!(
                            "ICE take {} ({:#x}, {}) bound to {:?} (sampled once: {:?})",
                            s.take,
                            s.take,
                            s.take_data
                                .as_ref()
                                .map_or("not found".into(), |t| format!("{:?} {:.2} s", t.name, t.length)),
                            s.bindings,
                            s.binding_flags
                        ),
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
                ItemKind::DynamicCamera(d) => format!(
                    "blend to {:?}: {}",
                    d.bindings,
                    d.channels.iter().map(|c| format!("{:?}", c.target)).collect::<Vec<_>>().join(", ")
                ),
                ItemKind::Teleport(t) => format!(
                    "teleport {:?} to trigger {} {:?} at {} mph{}",
                    t.racer,
                    t.trigger,
                    t.position.map(|v| v.round()),
                    t.speed_mph,
                    if t.resolved { "" } else { " (unresolved)" }
                ),
                ItemKind::DriveWithAi { max_speed_mph, style } => {
                    format!("AI drives the player (max {max_speed_mph} mph, style {style})")
                }
                ItemKind::AnimatedCar(c) => format!("{:?} follows {} ({} frames)", c.racer, c.label, c.keys.len()),
                ItemKind::HideRacecar(r) => format!("hide {r:?}"),
                ItemKind::HideHud => "hide HUD".into(),
                ItemKind::Letterbox(l) => format!("letterbox {:?}", l.bars),
                ItemKind::Layout { layout, layer, .. } => format!("layout {:?} layer {layer}", layout.map(|l| l.0)),
                ItemKind::SlowMo(m) => format!("slow motion {:.2} -> {:.2}", m.at(0.0), m.at(1.0)),
                ItemKind::EventTrigger => "event trigger".into(),
                ItemKind::ClearTraffic(c) => format!("clear traffic within {} m of {:?}", c.radius, c.points),
                ItemKind::Movie { movie, looping } => format!("movie {movie}{}", if *looping { " (loop)" } else { "" }),
                ItemKind::BlackQuad => "black quad".into(),
                ItemKind::StreamedImage { target, index } => format!("streamed image {target} <- {index}"),
                ItemKind::GeneralTrigger { name, trigger } => format!("general trigger {name} [{trigger}]"),
                ItemKind::Conductor(s) => format!("conductor: sequence {} ({} items)", s.unique_id, s.items.len()),
                ItemKind::Wave(w) => format!(
                    "wave {:?} gain {} fade in {} out {} type {}",
                    w.waves, w.gain.value, w.fade_in, w.fade_out, w.kind
                ),
                ItemKind::SetSongList { list, flags } => format!("set song list {list} {flags:?}"),
                ItemKind::MusicState(s) => format!("music state {s:?}"),
                ItemKind::VfxSpot(v) => {
                    format!("vfx spot {:?} at {:?} / {:?}", v.effect, v.locator_group, v.transform_binding)
                }
                ItemKind::Picture(p) => picture(p),
                ItemKind::Other(n) => n.clone(),
            };
            println!("    {:6.2}-{:6.2} {}{what}", i.start, i.end, "  ".repeat(i.depth as usize));
            // `MW_EVENTS_KEYS=<dir>`: the camera and car clips' keys, one CSV per item (for axis surveys).
            if let Some(dir) = std::env::var_os("MW_EVENTS_KEYS") {
                let (kind, keys, rate, extra) = match &i.kind {
                    ItemKind::AnimatedCamera(c) => {
                        ("cam", &c.keys, c.frame_rate, format!("{} {:.1}", c.relative_to, c.focal_length.at(0.0)))
                    }
                    ItemKind::AnimatedCar(c) => ("car", &c.keys, c.frame_rate, format!("{:?}", c.racer)),
                    _ => continue,
                };
                let path = Path::new(&dir).join(format!("{}_{label}_{:07.3}_{kind}.csv", e.id, i.start));
                let mut s = format!("# {} {} {rate} {extra}\n", i.start, i.end);
                for k in keys {
                    let [x, y, z] = k.translation;
                    let [a, b, c, d] = k.rotation;
                    s += &format!("{x} {y} {z} {a} {b} {c} {d}\n");
                }
                let _ = std::fs::write(path, s);
            }
        }
    }
}

/// `mwtool frontend`: the front end's data (`mw_assets::frontend`).
pub fn frontend(store: &AssetStore, all_spots: bool) -> Result<()> {
    use mw_assets::frontend as fe;
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).ok();
    let text = |id: u32| lang.as_ref().and_then(|l| l.get(id)).map_or_else(|| format!("#{id}"), str::to_owned);
    let start = store.start_config()?;
    println!("new game car: {:?}; default start: {:?}", start.default_vehicle, start.default_start);
    println!("{} swap spots (jack spots){}:", start.swap_spots.len(), if all_spots { "" } else { "; first 5" });
    for s in start.swap_spots.iter().take(if all_spots { usize::MAX } else { 5 }) {
        println!("  spot {} car {} location {} pose {:?}", s.id, s.vehicle, s.location, s.car);
    }
    let lc = store.loading_config()?;
    println!(
        "loading layouts {:?}, toggle layouts {:?}, {} pictures ({} s each), {} tips",
        lc.layouts,
        lc.toggle_layouts,
        lc.images.len(),
        lc.image_seconds,
        lc.tips.len()
    );
    for t in lc.tips.iter().take(3) {
        println!("  tip: {}", text(*t));
    }
    for (name, id) in [("StdData", 431_091), ("Data", 431_113)] {
        let items = store.menu_items(fe::PAUSE_SCREEN, id)?;
        let list: Vec<String> =
            items.iter().map(|(a, n, on)| format!("{a}={}{}", text(*n), if *on { "" } else { " (off)" })).collect();
        println!("pause menu {name}: {}", list.join(", "));
    }
    println!(
        "Press: {:?}, PressStart: {:?}, KEY_RETURN: {:?}",
        store.compliance_term("Press").map(text),
        store.compliance_term("PressStart").map(text),
        store.keyboard_button_text("KEY_RETURN").map(text)
    );
    for id in [fe::TITLE_SEQUENCE, fe::BOOT_TRANSITION_SEQUENCE] {
        let seq = store.ui_sequence(id)?;
        print_ui_sequence(&seq, 0);
        layout_shaders(store, &seq);
    }
    Ok(())
}

/// `mwtool frontend --sequence <id>`: one UI sequence.
pub fn print_sequence(store: &AssetStore, id: u32) -> Result<()> {
    let seq = store.ui_sequence(id)?;
    print_ui_sequence(&seq, 0);
    Ok(())
}

/// The shaders of the UI materials the sequence's layouts use (blend states, masks).
fn layout_shaders(store: &AssetStore, seq: &mw_assets::sequence::Sequence) {
    use mw_assets::sequence::ItemKind;
    for i in &seq.items {
        match &i.kind {
            ItemKind::Layout { layout: Some(id), .. } => {
                let Ok(l) = store.ui_layout(*id) else { continue };
                for e in &l.elements {
                    let shader = e.material.and_then(|m| store.ui_material_shader(m));
                    println!(
                        "  layout {} element {} {:?}: {}",
                        id.0 & 0xffff_ffff,
                        e.id,
                        e.kind,
                        shader.unwrap_or_default()
                    );
                }
            }
            ItemKind::Conductor(s) => layout_shaders(store, s),
            _ => {}
        }
    }
}

fn print_ui_sequence(seq: &mw_assets::sequence::Sequence, depth: usize) {
    use mw_assets::sequence::{Controller, ItemKind};
    let pad = "  ".repeat(depth);
    println!("{pad}sequence {} ({:.2} s, {} items)", seq.unique_id, seq.length, seq.items.len());
    for c in &seq.controllers {
        if let Controller::Jump { kind, trigger_time, destination, enabled, .. } = c {
            println!("{pad}  {kind}: at {trigger_time:.3} s -> {destination:.3} s if {enabled:?}");
        }
    }
    for i in &seq.items {
        let what = match &i.kind {
            ItemKind::Layout { layout, layer, texts, .. } => {
                format!("layout {} layer {layer} texts {texts:?}", layout.map_or(0, |l| l.0 & 0xffff_ffff))
            }
            ItemKind::Movie { movie, looping } => format!("movie {movie} loop {looping}"),
            ItemKind::BlackQuad => "black quad".into(),
            ItemKind::StreamedImage { target, index } => format!("streamed image {target} <- {index}"),
            ItemKind::GeneralTrigger { name, trigger } => format!("general trigger {name} [{trigger}]"),
            ItemKind::Conductor(_) => "conductor".into(),
            ItemKind::Wave(w) => format!("wave {:?} gain {} type {}", w.waves, w.gain.value, w.kind),
            ItemKind::SetSongList { list, flags } => format!("set song list {list} {flags:?}"),
            ItemKind::MusicState(s) => format!("music state {s:?}"),
            ItemKind::VfxSpot(v) => format!("vfx spot {:?}", v.effect),
            ItemKind::Other(n) => n.clone(),
            ItemKind::Camera(shots) => {
                let s: Vec<String> = shots
                    .iter()
                    .map(|s| format!("take {} {:?} bound {:?}", s.take, s.take_data.is_some(), s.bindings))
                    .collect();
                format!("camera [{}]", s.join("; "))
            }
            other => {
                // The variant's name (cameras, teleports, letterbox, ...).
                let d = format!("{other:?}");
                d[..d.find(|c: char| !c.is_alphanumeric()).unwrap_or(d.len())].to_string()
            }
        };
        let en = if i.enabled.is_empty() { String::new() } else { format!(" if {:?}", i.enabled) };
        println!("{pad}  {:7.3}-{:7.3} {what}{en}", i.start, i.end);
        if let ItemKind::Conductor(s) = &i.kind {
            print_ui_sequence(s, depth + 2);
        }
    }
}

/// One line for a picture / environment item (`mw_assets::sequence_picture`).
fn picture(p: &mw_assets::sequence_picture::PictureItem) -> String {
    use mw_assets::sequence_picture::PictureItem as P;
    let id = |h: &Option<mw_bundle::ResourceId>| h.map_or("-".to_string(), |h| format!("{:x}", h.0 & 0xFF_FFFF));
    match p {
        P::PostFx(f) => format!(
            "PostFX state {}{}{}{}",
            id(&f.state),
            if f.set_intensity { " (intensity curve)" } else { "" },
            if f.set_transition { " (transition curve)" } else { "" },
            if f.mode != 0 { format!(" (vector mode {})", f.mode) } else { String::new() }
        ),
        P::ImagePalette { palette } => format!("image palette {palette}"),
        P::DisableWorldRendering { restore } => format!("disable world rendering (restore {restore})"),
        P::SetTimeOfDay(t) => format!(
            "set time of day {:02}:{:02}:{:02} (time0 {:02}:{:02}:{:02}, flags {:#x}, {})",
            t.time1.hours,
            t.time1.minutes,
            t.time1.seconds,
            t.time0.hours,
            t.time0.minutes,
            t.time0.seconds,
            t.flags,
            t.d6529487
        ),
        P::EnvironmentTimeline { timeline } => format!("environment timeline {}", id(timeline)),
        other => other.name().to_string(),
    }
}
