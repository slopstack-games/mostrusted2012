//! Roadblocks in the runtime: the coordinator's roadblock task (`mw_police::coordinator::RoadblockTask`, run by
//! `police.rs`) picks a roadblock trigger region on the player's route; this module lays the roadblock out with the
//! ported roadblock manager (`mw_police::roadblock`: `FUN_00844ff0`, `FUN_00841710`, `FUN_0083f120`) and places its
//! items: cop cars (the layer item's `AIPlayerType` → `Rollout` → vehicle, `FUN_00845270`, parked) and world objects
//! (cones and barriers: the `PhysicalDefinition`'s `WorldObject` in `HAWAII/GLOBALRESOURCES.BNDL`, `FUN_00841390`)
//! through the loader. It feeds the crate the standing roadblock (`RoadState::roadblocks`: sight and the cooldown's
//! "found again", `FUN_00657ee0` / `FUN_0065c970`) and whether the player touches one of its cars
//! (`touches_roadblock`, `FUN_0065b930`), and removes it once it lies behind the player farther than
//! `GameLogicConfig` +0x280 (`FUN_008414f0`) or after a bust (HSM "Do Pursuits" `RemoveRoadblocks`).
//!
//! Model decisions (docs/decomp.md "Roadblocks"): the player's predicted route (coordinator +0x31c) is a road-graph
//! walk along the car's heading (the game's route predictor was not read); an item stands where a ray from 4 m above
//! to 10 m below its point meets the world, upright on the hit normal; roadblock cars are parked (handbrake, no
//! driver: what the game's AI does with them was not read), without light bars or sirens; spike strips are not
//! placed.
//!
//! `MW_ROADBLOCK=<definition>` places that roadblock at the nearest trigger region ahead of the player at start (for
//! screenshots).

use std::sync::Arc;

use avian3d::prelude::*;
use bevy::prelude::*;
use mw_assets::police::{RoadblockData, RoadblockItemKind};
use mw_police::coordinator::TriggerRegion;
use mw_police::roadblock::{self as rb, RoadWalk};

use crate::drive::{Car, CarBuild, CarControls, CarParts, Player};
use crate::loader::{CarRequest, CarSpec, LoaderRequests, TestProp};

/// Cop-car slots (less `police::SLOT_BASE`) from here up are roadblock cars.
pub const SLOT: usize = 1_000_000;
/// MODEL: the predicted route reaches this far (past the task's 600 m window, `AIConfig` +0x1a8).
const ROUTE_LENGTH: f32 = 700.0;

/// A roadblock's world object (tagged with its roadblock).
#[derive(Component)]
pub struct RoadblockPiece(pub u32);

/// A roadblock's cop car.
#[derive(Component)]
pub struct RoadblockCar(pub u32);

/// A roadblock trigger region (`TriggerData` kind 0x12, a box): the ground point under its centre, its z axis and
/// its x size (`FUN_005226b0`, `FUN_0042f250`).
#[derive(Debug, Clone, Copy)]
struct Region {
    ground: Vec3,
    dir: Vec3,
    width: f32,
}

struct Standing {
    tag: u32,
    position: Vec3,
    definition: u32,
    cars: Vec<Entity>,
}

#[derive(Resource, Default)]
pub struct Roadblocks {
    data: Option<Arc<RoadblockData>>,
    regions: Vec<Region>,
    /// The regions as the coordinator's task reads them.
    pub triggers: Vec<TriggerRegion>,
    /// The player's predicted route this tick (node positions from the player on).
    pub route: Vec<Vec3>,
    /// The standing roadblock's point (none or one: `FUN_00841040` takes a record only while none stands).
    pub standing: Vec<Vec3>,
    /// The task's pick (trigger region, definition), placed by [`place_roadblocks`].
    ordered: Option<(usize, u32)>,
    current: Option<Standing>,
    /// Cars on order: slot → pose.
    pending: Vec<(usize, u32, Vec3, f32)>,
    /// Cars back from the loader (`police::spawn_cops` hands them over).
    pub arrived: Vec<(usize, Box<CarSpec>, CarParts)>,
    /// After a bust (`RemoveRoadblocks`).
    pub remove_all: bool,
    next_tag: u32,
    next_slot: usize,
    /// For the log / test: roadblocks placed, cars and objects requested.
    pub placed: u32,
    pub cars: u32,
    pub objects: u32,
}

impl Roadblocks {
    /// The coordinator's task asked for a roadblock at region `k` (`FUN_005226b0` → world message 7).
    pub fn order(&mut self, pick: (usize, u32)) {
        if self.current.is_none() && self.ordered.is_none() {
            self.ordered = Some(pick);
        }
    }
}

pub fn load_roadblocks(game: Res<crate::Game>, mut roadblocks: ResMut<Roadblocks>) {
    match game.store.roadblock_data() {
        Ok(d) => {
            info!(
                "roadblocks: {} definitions, {} layers, {} item kinds",
                d.definitions.len(),
                d.layers.len(),
                d.items.len()
            );
            roadblocks.data = Some(Arc::new(d));
        }
        Err(e) => warn!("roadblocks: no data ({e})"),
    }
    let Ok(td) = game.store.trigger_data() else { return };
    for t in td.triggers.iter().filter(|t| t.kind == 0x12) {
        let mw_formats::TriggerShape::Box { rotation: [x, y, z, w], size } = t.shape else { continue };
        let q = Quat::from_xyzw(x, y, z, w);
        let centre = Vec3::from(t.position);
        roadblocks.regions.push(Region {
            ground: centre - Vec3::Y * (size[1] * 0.5),
            dir: (q * Vec3::Z).normalize_or(Vec3::Z),
            width: size[0],
        });
        // `FUN_0042c0b0`: a box's bounding radius is the length of half its size.
        roadblocks.triggers.push(TriggerRegion { position: centre, radius: (Vec3::from(size) * 0.5).length() });
    }
    info!("roadblocks: {} trigger regions", roadblocks.regions.len());
    if let Some(def) = std::env::var("MW_ROADBLOCK").ok().and_then(|v| v.parse::<u32>().ok()) {
        roadblocks.ordered = Some((usize::MAX, def));
    }
}

/// The player's predicted route (coordinator +0x31c; MODEL: a road-graph walk along the car's heading).
pub fn predicted_route(graph: &mw_formats::AiData, player: &Transform, out: &mut Vec<Vec3>) {
    out.clear();
    out.push(player.translation);
    let f = player.rotation * Vec3::Z;
    crate::police::walk(graph, player.translation, Vec2::new(f.x, f.z).normalize_or(Vec2::Y), |n, along, _| {
        out.push(Vec3::from(graph.nodes[n].position));
        along < ROUTE_LENGTH
    });
}

/// `FUN_0083fad0` on the runtime's road graph.
struct Road<'a>(&'a mw_formats::AiData);

impl RoadWalk for Road<'_> {
    fn walk(&self, from: Vec3, dir: Vec3, distance: f32) -> Option<(Vec3, Vec3)> {
        let g = self.0;
        let node = |i: usize| Vec3::from(g.nodes[i].position);
        let n = g.nearest_node(from.to_array(), false)?;
        // The segment nearest the point (`FUN_004375b0`), oriented along `dir`.
        let (mut a, mut b) = g.nodes[n].links.iter().map(|&l| l as usize).map(|m| (n, m)).min_by(|x, y| {
            let d = |(p, q): (usize, usize)| {
                mw_police::coordinator::closest_on_segment(node(p), node(q), from).distance_squared(from)
            };
            d(*x).total_cmp(&d(*y))
        })?;
        if (node(b) - node(a)).dot(dir) < 0.0 {
            std::mem::swap(&mut a, &mut b);
        }
        let q = mw_police::coordinator::closest_on_segment(node(a), node(b), from);
        let mut left = distance + q.distance(node(a));
        for _ in 0..256 {
            let seg = node(b) - node(a);
            let len = seg.length();
            let d = seg.normalize_or(dir);
            if left < len {
                return Some((node(a) + d * left, d));
            }
            left -= len;
            let Some(next) = g.next_node(b, [d.x, d.z], Some(a)) else { return Some((node(b), d)) };
            (a, b) = (b, next);
        }
        None
    }
}

/// Place the ordered roadblock, build its arrived cars, remove it when the player has left it behind.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn place_roadblocks(
    mut commands: Commands,
    mut roadblocks: ResMut<Roadblocks>,
    police: Res<crate::police::Police>,
    requests: Option<Res<LoaderRequests>>,
    spatial: SpatialQuery,
    player: Query<&Transform, (With<Player>, With<Car>)>,
    pieces: Query<(Entity, &RoadblockPiece)>,
    cars: Query<(Entity, &RoadblockCar)>,
) {
    let rbs = &mut *roadblocks;
    let Ok(pt) = player.single() else { return };
    let Some(data) = rbs.data.clone() else { return };

    // Removal (`FUN_008414f0`): behind the player and beyond `GameLogicConfig` +0x280 (× 0.8 outside a pursuit),
    // or after a bust.
    if let Some(s) = &rbs.current {
        let far = rb::removal_distance(&data, police.pursuit.in_chase_or_cooldown());
        if rbs.remove_all || rb::passed(s.position, pt.translation, pt.rotation * Vec3::Z, far) {
            let tag = s.tag;
            info!("roadblock {tag} (definition {}) removed", s.definition);
            for (e, c) in &cars {
                if c.0 == tag {
                    commands.entity(e).despawn();
                }
            }
            rbs.pending.clear();
            rbs.arrived.clear();
            rbs.current = None;
        }
    }
    rbs.remove_all = false;
    // Objects of any roadblock but the standing one: those of a removed roadblock, including the ones still
    // loading when it was removed (they arrive later with its tag and nothing else would remove them).
    let live = rbs.current.as_ref().map(|s| s.tag);
    for (e, p) in &pieces {
        if Some(p.0) != live {
            commands.entity(e).despawn();
        }
    }

    // The cars that have come back from the loader: parked where the layout put them.
    for (slot, spec, parts) in std::mem::take(&mut rbs.arrived) {
        let Some(i) = rbs.pending.iter().position(|p| p.0 == slot) else { continue };
        let (_, tag, start, heading) = rbs.pending.swap_remove(i);
        let car = crate::drive::build_car(
            &mut commands,
            &CarBuild {
                name: &format!("roadblock car {slot}"),
                spec: &spec,
                parts: &parts,
                handling: &spec.handling,
                nitrous_rules: None,
                start,
                heading,
                velocity: Vec3::ZERO,
                damage: Some(crate::damage::DamageSource(spec.damage.clone().map(|d| (d, spec.impact_graph)))),
            },
        );
        commands
            .entity(car)
            .insert((RoadblockCar(tag), CarControls { brake: 1.0, handbrake: true, ..Default::default() }));
        if let Some(s) = rbs.current.as_mut().filter(|s| s.tag == tag) {
            s.cars.push(car);
        }
    }

    // A new roadblock (`FUN_00847af0` states 0–6).
    let Some((k, definition)) = rbs.ordered.take() else { return };
    let Some(graph) = police.graph() else { return };
    let region = match rbs.regions.get(k) {
        Some(r) => *r,
        // `MW_ROADBLOCK`: the nearest region ahead of the player.
        None => {
            let fwd = pt.rotation * Vec3::Z;
            let mut near: Vec<&Region> = rbs.regions.iter().collect();
            near.sort_by(|a, b| a.ground.distance(pt.translation).total_cmp(&b.ground.distance(pt.translation)));
            for r in near.iter().take(3) {
                info!("roadblock region at {:.0?}, z axis {:.2?}, {:.1} m wide", r.ground, r.dir, r.width);
            }
            let Some(r) = rbs
                .regions
                .iter()
                .filter(|r| (r.ground - pt.translation).dot(fwd) > 30.0)
                .min_by(|a, b| a.ground.distance(pt.translation).total_cmp(&b.ground.distance(pt.translation)))
            else {
                return;
            };
            *r
        }
    };
    let dir = rb::facing_away(region.ground, region.dir, pt.translation);
    let items = rb::layout(&data, definition, region.ground, dir, region.width, &Road(graph));
    // The ground probe of each item (`FUN_00841710`).
    let ground = SpatialQueryFilter::default().with_mask(crate::props::GameLayer::World);
    let mut grounded = Vec::with_capacity(items.len());
    for it in &items {
        let Some(&kind) = data.items.get(&it.object) else { continue };
        let top = it.position + Vec3::Y * rb::PROBE_UP;
        let Some(hit) = spatial.cast_ray(top, Dir3::NEG_Y, rb::PROBE_UP + rb::PROBE_DOWN, true, &ground) else {
            continue;
        };
        let (_, _, fwd) = rb::item_frame(it.dir, hit.normal.normalize_or(Vec3::Y), it.angle);
        grounded.push((kind, top - Vec3::Y * hit.distance, fwd.x.atan2(fwd.z)));
    }
    if grounded.is_empty() {
        // The world under it is not loaded yet: again next frame.
        if !items.is_empty() {
            rbs.ordered = Some((k, definition));
        }
        return;
    }
    let tag = rbs.next_tag;
    rbs.next_tag += 1;
    rbs.placed += 1;
    let mut n_cars = 0;
    for &(kind, at, heading) in &grounded {
        let Some(requests) = &requests else { continue };
        let req = match kind {
            RoadblockItemKind::Car { vehicle } => {
                if n_cars >= rb::MAX_CARS {
                    continue;
                }
                n_cars += 1;
                let slot = crate::police::SLOT_BASE + SLOT + rbs.next_slot;
                rbs.next_slot += 1;
                rbs.pending.push((slot - crate::police::SLOT_BASE, tag, at, heading));
                rbs.cars += 1;
                CarRequest {
                    bundle: format!("VEHICLES/VEH_{vehicle}_HI.BNDL"),
                    paint_index: 0,
                    traffic: None,
                    opponent: Some(slot),
                    prop: None,
                }
            }
            RoadblockItemKind::Object { world_object } => {
                rbs.objects += 1;
                let prop = TestProp {
                    object: world_object,
                    kind: mw_assets::props::ObjectKind::Dynamic,
                    position: at,
                    yaw: heading,
                };
                CarRequest {
                    bundle: String::new(),
                    paint_index: 0,
                    traffic: None,
                    opponent: None,
                    prop: Some((prop, tag)),
                }
            }
            RoadblockItemKind::SpikeStrip => continue,
        };
        let _ = requests.0.send(req);
    }
    info!(
        "roadblock {tag}: definition {definition} at {:.0?} facing {:.2?}, {:.1} m wide: {} items ({} on the ground, {n_cars} cars), {:.0} m from the player",
        region.ground,
        dir,
        region.width,
        items.len(),
        grounded.len(),
        region.ground.distance(pt.translation)
    );
    rbs.current = Some(Standing { tag, position: region.ground, definition, cars: Vec::new() });
    rbs.standing = vec![region.ground];
}

/// Whether the player's car touches one of the roadblock's cars (`FUN_0065b930`; the contact test is the cops'
/// MODEL box test, `police::touching`).
pub fn touches(pt: &Transform, pcar: &Car, cars: &[(&Transform, &Car)]) -> bool {
    cars.iter().any(|(t, c)| crate::police::touching(pt, pcar, t, c))
}

impl Roadblocks {
    /// Forget the standing roadblock's point once it is gone.
    pub fn sync_standing(&mut self) {
        if self.current.is_none() {
            self.standing.clear();
        }
    }
}
