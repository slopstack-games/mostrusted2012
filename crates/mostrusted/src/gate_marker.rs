//! Race checkpoint markers: the points of interest the race code puts on the gates (decomp.md "Races",
//! "Markers"), drawn as the game's points-of-interest widget draws a point's world layouts.
//!
//! `FUN_00759860` registers, when the player's target gate changes, a point of interest at the target gate and
//! (while the target is not the last entry) one at the gate after it. `FUN_007545b0` picks the look by key:
//! `CheckPointsConfig.NormalPoi` for the target, its +0x24 for the next gate, `MapPoiConfig` +0x28 / +0x2c
//! when that gate is the finish. `FUN_00749930` gives the transform: the gate's rotation at its ground point
//! (box centre lowered by half its height). The race HUD screen's `Widget_PointOfInterest` lists the looks
//! (`PointOfInterestVisuals`, `mw_assets::ui::PoiVisual`); each has world layouts with instance transforms
//! S(0.1) · S(`DistanceToCamera` 20…500 m → 0.5…8) · R(`Rot`) · T(`Pos`) · T(0, 0.25, 0), so a marker is a UI
//! panel in the gate's plane, 0.1 m per layout pixel, growing with distance:
//! - target gate: a 4 × 115 px pole (`UIDepthTestedShader`) and "CHECK" over the race time on glow strips,
//!   faded in from 20 to 60 m away (`Colour_Transforms`);
//! - next gate: a thin 2 × 100 px white bar at 0.4 alpha and a faint "CHECK", faded out within 10 m;
//! - the finish: the pole with "FINISH" and the time; as the next gate, the pole and a faint "FINISH".
//!
//! The binding evaluator for `Pos` / `Rot` / `DistanceToCamera` is `FUN_00805870`: `Rot` is the point matrix's
//! Euler angles in degrees through `FUN_005bad40`, so the gate's size scaling in the matrix drops out (exactly
//! for a level gate); `DistanceToCamera` is |camera − point|. Drawn by `world_ui.rs` after the tonemap with a
//! depth test, as `UIDepthTestedShader` does.
//!
//! Model decisions (not traced; listed in decomp.md): layout x runs to the approaching driver's right and
//! layout y down (the 3D UI's axis convention was not found; this stands the pole up and reads the text left
//! to right from the approach side); the rotation is the gate's quaternion itself (not rebuilt from Euler
//! angles, whose composition order in the UI transform is not known); colour transform type 3 multiplies the layout's opacity; the text (`FlaptGenericShader`)
//! is depth-tested like the rest; markers show in the intro and race phases while the HUD is shown (not under
//! a sequence's `HideHudSequenceItem`), until the finish.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use bevy::prelude::*;
use mw_assets::ui::{DrawOptions, PlacedLayout, UiAnim, UiBlend, UiScene};
use mw_bundle::ResourceId;
use mw_formats::binding::{BindValue, Binding, race_time};
use mw_formats::ui::LayoutInstance;

use crate::race::{RaceCheckpoint, RacePhase, RaceState, Racer, RacerProgress};
use crate::world_ui::{WorldUiBatch, WorldUiQuads, WorldUiVertex};

/// The screen whose `Widget_PointOfInterest` lists the looks: the offline race HUD.
const POI_SCREEN: &str = crate::hud::RACE_HUD;

pub struct GateMarkerPlugin;

impl Plugin for GateMarkerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_loading).add_systems(Update, (receive, draw_markers).chain());
    }
}

/// Which look a gate is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateLook {
    /// The gate being driven to (`NormalPoi`).
    Gate,
    /// The gate after it (`CheckPointsConfig` +0x24).
    GatePreview,
    /// The finish as the target (`MapPoiConfig` +0x28).
    Finish,
    /// The finish as the gate after the target (`MapPoiConfig` +0x2c).
    FinishPreview,
}

/// The gates `FUN_00759860` marks for a racer whose target is route entry `target`: the target (finish look
/// when it is the last entry) and, while the target is not the last entry, the one after it.
pub fn shown_gates(race: &RaceState, target: usize) -> Vec<(GateLook, RaceCheckpoint)> {
    let last = race.entries.len().saturating_sub(1);
    let mut out = Vec::new();
    if target > last {
        return out;
    }
    for (entry, active) in [(target, true), (target + 1, false)] {
        if entry > last {
            continue;
        }
        let Some(gate) = race.entries.get(entry).and_then(|e| e.gate).and_then(|g| race.checkpoints.get(g)) else {
            continue;
        };
        let look = match (entry == last, active) {
            (true, true) => GateLook::Finish,
            (true, false) => GateLook::FinishPreview,
            (false, true) => GateLook::Gate,
            (false, false) => GateLook::GatePreview,
        };
        out.push((look, *gate));
    }
    out
}

struct Loaded {
    scene: UiScene,
    looks: Vec<GateLook>,
    instances: Vec<Option<LayoutInstance>>,
    images: Vec<(ResourceId, Image)>,
}

#[derive(Resource)]
struct Loading(Mutex<Receiver<Loaded>>);

#[derive(Resource)]
pub(crate) struct GateMarkers {
    scene: UiScene,
    /// The look and the instance (world transforms) of each layout of `scene`.
    looks: Vec<GateLook>,
    instances: Vec<Option<LayoutInstance>>,
    textures: HashMap<ResourceId, Handle<Image>>,
    anim: UiAnim,
    /// Per (layout, timeline): the trigger's value and when it last changed (s).
    triggers: HashMap<(usize, usize), (bool, f64)>,
}

fn start_loading(mut commands: Commands, game: Res<crate::Game>) {
    let store = game.store.clone();
    let (tx, rx) = channel();
    crate::platform::thread::spawn(move || {
        let (keys, pois, lang) =
            match (store.poi_keys(), store.ui_poi_visuals(POI_SCREEN), store.language(mw_assets::DEFAULT_LANGUAGE)) {
                (Ok(k), Ok(p), Ok(l)) => (k, p, l),
                (k, p, l) => {
                    warn!("gate markers: no point-of-interest looks ({:?} / {:?} / {:?})", k.err(), p.err(), l.err());
                    return;
                }
            };
        let mut layouts: Vec<PlacedLayout> = Vec::new();
        let mut looks = Vec::new();
        for (look, key) in [
            (GateLook::Gate, keys.gate),
            (GateLook::GatePreview, keys.gate_preview),
            (GateLook::Finish, keys.finish),
            (GateLook::FinishPreview, keys.finish_preview),
        ] {
            let Some(poi) = pois.iter().find(|p| p.key == key) else {
                warn!("gate markers: no look for {look:?} (point of interest {key})");
                continue;
            };
            for l in &poi.world {
                layouts.push(l.clone());
                looks.push(look);
            }
        }
        // The scene draws the layouts without their instances (in layout pixels; `UiScene` would read the first
        // translate as a screen anchor); `draw_markers` applies the instance transforms in the world.
        let instances = layouts.iter_mut().map(|l| l.instance.take()).collect();
        let scene = store.ui_scene(layouts, &lang);
        let images = scene
            .textures()
            .into_iter()
            .filter_map(|id| Some((id, crate::game_text::decode_texture(&store, id)?)))
            .collect();
        info!("gate markers: {} world layouts", scene.layouts.len());
        let _ = tx.send(Loaded { scene, looks, instances, images });
    });
    commands.insert_resource(Loading(Mutex::new(rx)));
}

fn receive(mut commands: Commands, loading: Option<Res<Loading>>, mut images: ResMut<Assets<Image>>) {
    let Some(loading) = loading else { return };
    let Ok(loaded) = loading.0.lock().unwrap().try_recv() else { return };
    commands.remove_resource::<Loading>();
    let textures = loaded.images.into_iter().map(|(id, img)| (id, images.add(img))).collect();
    commands.insert_resource(GateMarkers {
        scene: loaded.scene,
        looks: loaded.looks,
        instances: loaded.instances,
        textures,
        anim: UiAnim::default(),
        triggers: HashMap::new(),
    });
}

/// A layout instance's `FADE(delay, duration, IN|OUT)` step `since` ms after it started: the opacity it leaves.
fn fade(timeline: &str, since: f32) -> f32 {
    let Some(args) =
        timeline.trim().strip_prefix("FADE").map(|s| s.trim().trim_start_matches('(').trim_end_matches(')'))
    else {
        return 1.0;
    };
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    let num = |i: usize| parts.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
    let (delay, duration) = (num(0), num(1));
    let p = if duration <= 0.0 {
        f32::from(u8::from(since >= delay))
    } else {
        ((since - delay) / duration).clamp(0.0, 1.0)
    };
    match parts.get(2).copied() {
        Some("OUT") => 1.0 - p,
        _ => p,
    }
}

/// The instance's transforms in order, as a function from a layout point (reference pixels) to the world.
fn placement(
    instance: Option<&LayoutInstance>,
    position: Vec3,
    rotation: Quat,
    lookup: &mut dyn FnMut(&str, Option<&str>) -> Option<BindValue>,
) -> Vec<(u16, Vec3, Quat)> {
    let mut ops = Vec::new();
    for t in instance.map(|i| i.transforms.as_slice()).unwrap_or_default() {
        let bound = (!t.binding.is_empty()).then(|| Binding::parse(&t.binding).and_then(|b| b.eval(lookup))).flatten();
        match (t.kind, t.binding.as_str()) {
            (2, "Rot") => ops.push((2, Vec3::ZERO, rotation)),
            (1, "Pos") => ops.push((1, position, Quat::IDENTITY)),
            (3, b) if !b.is_empty() => {
                if let Some(v) = bound {
                    ops.push((3, Vec3::splat(v.num() as f32), Quat::IDENTITY));
                }
            }
            (3, _) => ops.push((3, Vec3::from_array(t.value), Quat::IDENTITY)),
            (2, "") => {
                let [x, y, z] = t.value.map(f32::to_radians);
                ops.push((2, Vec3::ZERO, Quat::from_euler(EulerRot::XYZ, x, y, z)));
            }
            (1, "") => ops.push((1, Vec3::from_array(t.value), Quat::IDENTITY)),
            _ => {}
        }
    }
    ops
}

fn apply(ops: &[(u16, Vec3, Quat)], p: [f32; 2]) -> Vec3 {
    // Model decision: layout x → the approaching driver's right (the gate's −X), layout y (down) → −Y.
    let mut v = Vec3::new(-p[0], -p[1], 0.0);
    for &(kind, vec, rot) in ops {
        v = match kind {
            3 => v * vec,
            2 => rot * v,
            _ => v + vec,
        };
    }
    v
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_markers(
    markers: Option<ResMut<GateMarkers>>,
    race: Option<Res<RaceState>>,
    racers: Query<(&Racer, &RacerProgress)>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    hud: Option<Res<crate::hud::HudSettings>>,
    time: Res<Time>,
    mut out: ResMut<WorldUiQuads>,
) {
    out.0.clear();
    let (Some(mut m), Some(race)) = (markers, race) else { return };
    let shown = matches!(race.phase, RacePhase::Intro | RacePhase::Racing)
        && hud.is_none_or(|h| h.enabled && !h.hidden_by_sequence);
    let Some(cam) = camera.iter().next().map(|c| c.translation()) else { return };
    if !shown {
        return;
    }
    let target = racers.iter().find(|(r, _)| r.player).map_or(1, |(_, p)| p.target());
    let now = time.elapsed_secs_f64();
    let GateMarkers { scene, looks, instances, textures, anim, triggers } = &mut *m;
    let mut batches: Vec<WorldUiBatch> = Vec::new();
    let mut batch_of: HashMap<(Option<ResourceId>, bool), usize> = HashMap::new();
    for (look, gate) in shown_gates(&race, target) {
        let position = gate.ground();
        let rotation = gate.transform.rotation;
        let distance = cam.distance(position) as f64;
        let clock = race.clock as f64;
        let mut lookup = |path: &str, _arg: Option<&str>| -> Option<BindValue> {
            Some(match path {
                "DistanceToCamera" => distance.into(),
                "Mode.Time" => race_time(clock).into(),
                "Mode.IsScoreBasedEvent" | "Mode.IsInGameIntroFreedrive" | "Players.LocalPlayer.Blinded" => {
                    false.into()
                }
                "Mode.IsRaceEvent" => true.into(),
                _ => return None,
            })
        };
        for (i, _) in looks.iter().enumerate().filter(|(_, l)| **l == look) {
            let instance = instances.get(i).and_then(Option::as_ref);
            let mut out = WorldLayoutOut { batches: &mut batches, batch_of: &mut batch_of, textures };
            draw_world_layout(scene, i, instance, position, rotation, &mut lookup, anim, triggers, now, &mut out);
        }
    }
    if std::env::var_os("MW_GATE_LOG").is_some() && (now * 2.0).floor() != ((now - time.delta_secs_f64()) * 2.0).floor()
    {
        for (look, gate) in shown_gates(&race, target) {
            info!(
                "gate marker {look:?} at {:.1} up {:.2} (camera {:.0} m), {} triangles",
                gate.ground(),
                gate.transform.rotation * Vec3::Y,
                cam.distance(gate.ground()),
                batches.iter().map(|b| b.vertices.len() / 3).sum::<usize>()
            );
        }
    }
    out.0 = batches;
}

/// Where [`draw_world_layout`] puts its triangles: batches by (texture, additive).
pub(crate) struct WorldLayoutOut<'a> {
    pub batches: &'a mut Vec<WorldUiBatch>,
    pub batch_of: &'a mut HashMap<(Option<ResourceId>, bool), usize>,
    pub textures: &'a HashMap<ResourceId, Handle<Image>>,
}

/// Draw layout `i` of `scene` as a point of interest's world layout at `position` / `rotation`: its instance's
/// visibility binding, colour transforms (type 3: opacity) and triggered timelines (`FADE`), then its elements
/// through the instance transforms (`placement`). `triggers` keeps each (layout, timeline)'s value and when it
/// last changed; a caller drawing several points with one scene keeps one map per point.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_world_layout(
    scene: &UiScene,
    i: usize,
    instance: Option<&LayoutInstance>,
    position: Vec3,
    rotation: Quat,
    lookup: &mut dyn FnMut(&str, Option<&str>) -> Option<BindValue>,
    anim: &mut UiAnim,
    triggers: &mut HashMap<(usize, usize), (bool, f64)>,
    now: f64,
    out: &mut WorldLayoutOut,
) {
    // The layout's opacity: visibility, colour transforms and triggered timelines.
    let mut alpha = 1.0f32;
    if let Some(inst) = instance {
        if !inst.visibility.is_empty()
            && !Binding::parse(&inst.visibility).and_then(|b| b.eval(&mut *lookup)).is_some_and(|v| v.truthy())
        {
            return;
        }
        for c in inst.colours.iter().filter(|c| c.kind == 3) {
            let v = Binding::parse(&c.binding).and_then(|b| b.eval(&mut *lookup)).map_or(c.value, |v| v.num() as f32);
            alpha *= v.clamp(0.0, 1.0);
        }
        for (k, tl) in inst.timelines.iter().enumerate() {
            let on = Binding::parse(&tl.trigger).and_then(|b| b.eval(&mut *lookup)).is_some_and(|v| v.truthy());
            let entry = triggers.entry((i, k)).or_insert((on, f64::NEG_INFINITY));
            if entry.0 != on {
                *entry = (on, now);
            }
            let since = ((now - entry.1) * 1000.0).min(f32::MAX as f64) as f32;
            alpha *= fade(if on { &tl.on } else { &tl.off }, since);
        }
    }
    if alpha <= 0.001 {
        return;
    }
    let ops = placement(instance, position, rotation, lookup);
    let mut options = DrawOptions::new(Some(Box::new(move |j| j == i)), Some(anim), now);
    for q in scene.draw_with([1280.0, 720.0], lookup, &mut options) {
        let key = (q.texture, q.blend == UiBlend::Additive);
        let b = *out.batch_of.entry(key).or_insert_with(|| {
            out.batches.push(WorldUiBatch {
                texture: q.texture.and_then(|t| out.textures.get(&t).cloned()),
                additive: q.blend == UiBlend::Additive,
                vertices: Vec::new(),
            });
            out.batches.len() - 1
        });
        if q.texture.is_some() && out.batches[b].texture.is_none() {
            continue;
        }
        let [x, y, w, h] = q.rect;
        let (cx, cy) = (x + w * 0.5, y + h * 0.5);
        let (s, c) = q.rotation.to_radians().sin_cos();
        let corner = |dx: f32, dy: f32| {
            let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
            apply(&ops, [cx + rx, cy + ry])
        };
        let [u0, v0, u1, v1] = q.uv;
        let colour = [q.colour[0], q.colour[1], q.colour[2], q.colour[3] * alpha];
        let v = |p: Vec3, uv: [f32; 2]| WorldUiVertex { position: p.to_array(), uv, colour };
        let (a, bb, cc, d) = (
            v(corner(-w * 0.5, -h * 0.5), [u0, v0]),
            v(corner(w * 0.5, -h * 0.5), [u1, v0]),
            v(corner(w * 0.5, h * 0.5), [u1, v1]),
            v(corner(-w * 0.5, h * 0.5), [u0, v1]),
        );
        out.batches[b].vertices.extend([a, bb, cc, a, cc, d]);
    }
}
