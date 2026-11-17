//! The Bear actions of the cars' collision effects (`particles.rs` child module): the collision-mesh emitter
//! (0x01, `FUN_0044bf90`), the 3-D rotations (0x0B `FUN_00453070`, 0x2D `FUN_0045ddf0`) and the world collision
//! of rigid debris (0x40, `FUN_00462750`). Formats and constants: `mw_formats::bear` (`MeshEmitter`,
//! `Op::Rotation3dInit`, `Op::Rotation3d`, `Op::WorldCollide`); docs/decomp.md "Bear particle runtime".

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::Arc;

use bevy::prelude::*;
use mw_formats::bear::{Layer, MeshEmitter, Op, Spawner};

use super::{LayerSim, Params, Particle, Rng, birth};

/// The emitter's carried fraction at the start and whenever it is not emitting (`.rdata` 0x00f9d468).
pub(super) const CARRY_START: f32 = 0.9999;

/// A ray from `from` to `to` against the world: the hit's fraction of the segment and the surface normal.
pub(crate) type WorldCast<'a> = &'a dyn Fn(Vec3, Vec3) -> Option<(f32, Vec3)>;

/// `CollisionMeshParams` (the param block's +0x30 mesh handle, +0x40 point, +0x50 normal): which collision mesh
/// and the contact in its (the car's) space. `normal` points from the car towards what it hit (model decision:
/// the sign of the game's contact normal was not traced; this way the struck face passes the facing test).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MeshContact {
    pub mesh: u32,
    pub point: Vec3,
    pub normal: Vec3,
}

/// The cars' collision meshes (`AssetStore::vehicle_collision_mesh`), by the index [`MeshContact`] carries.
#[derive(Resource, Default)]
pub(crate) struct CollisionMeshes {
    list: Vec<Arc<Vec<[Vec3; 3]>>>,
}

impl CollisionMeshes {
    pub(crate) fn add(&mut self, tris: Vec<[Vec3; 3]>) -> u32 {
        self.list.push(Arc::new(tris));
        self.list.len() as u32 - 1
    }

    pub(super) fn get(&self, i: u32) -> Option<&[[Vec3; 3]]> {
        self.list.get(i as usize).map(|t| t.as_slice())
    }
}

/// `EmitterCollisionMesh` (run `FUN_0044bf90`): particles on the triangles of the car's collision mesh around the
/// contact point, `area × (burst · slot + rate · dt · slot) × camera fade` per triangle (the fraction carried from
/// triangle to triangle and rounded to nearest). Each particle sits at `v0 + a·(v2 − v0) + b·(1 − a)·(v1 − v0)`
/// (`a`, `b` uniform) and leaves at polar angle `θ` (turns) from the triangle's normal `n̂ = normalize((v1 − v0) ×
/// (v2 − v0))` about a uniform azimuth `φ ∈ [−½, ½)`: with `r = min(|φ|, ½ − |φ|)`, `θ = s·(c0 + r·c2) + s·(c1 +
/// r·c3)·u` (`c` = the compiled angle words, `s` = the angle slot, `u` uniform); direction `sin φ sin θ · (t × n̂) +
/// cos φ sin θ · t + cos θ · n̂` with `t = normalize(n̂ × (v1 − v0))`, speed `rand(speed) · slot`. Positions and
/// velocities are in mesh space; the secondaries move them into the effect's frame (`Transform` = the car).
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn(
    layer: &Layer,
    sim: &mut LayerSim,
    sp: &Spawner,
    me: &MeshEmitter,
    secondaries: &[u8],
    p: &Params,
    contact: MeshContact,
    tris: &[[Vec3; 3]],
    eye: Vec3,
    dt: f32,
    rng: &mut Rng,
) {
    if !p.emitting {
        sim.mesh_carry = CARRY_START;
        return;
    }
    sim.mesh_clock += dt;
    let t = sim.mesh_clock;
    if t < me.time_window[0] || (t > me.time_window[1] && sim.mesh_burst_spent) {
        return;
    }
    let burst = if sim.mesh_burst_spent { 0.0 } else { sp.burst as f32 };
    sim.mesh_burst_spent = true;
    let [s_radius, s_rate, s_burst, s_speed, s_angle] = me.slots.map(|i| sim.slot(i));
    let per_area = burst * s_burst + sp.rate * dt * s_rate;
    if per_area <= 0.0 {
        return;
    }
    // `FUN_0043ecd0`: the camera-distance fade over the spawner's LOD window.
    let origin = p.transform.w_axis.truncate();
    let fade = if sp.lod[1] > sp.lod[0] {
        1.0 - ((origin.distance(eye) - sp.lod[0]) / (sp.lod[1] - sp.lod[0])).clamp(0.0, 1.0)
    } else {
        1.0
    };
    // The compiled angle words (`FUN_00449630`): turns, the ranges × 4.
    let [[a0, a1], [b0, b1]] = sp.angles;
    let c0 = a0 / 360.0;
    let c1 = 4.0 * (a1 - a0) / 360.0;
    let c2 = b0 / 360.0 - c0;
    let c3 = 4.0 * (b1 - b0) / 360.0 - c1;
    let radius = me.radius * s_radius;
    let (point, normal) = (contact.point, contact.normal);
    let overwrite = layer.flags & 2 != 0;
    for &[v0, v1, v2] in tris {
        let centroid = (v0 + v1 + v2) / 3.0;
        let d = centroid - point;
        if d.length() > radius || d.dot(normal) > me.plane {
            continue;
        }
        let e1 = v1 - v0;
        let e2 = v2 - v0;
        let cross = e1.cross(e2);
        let Some(n) = cross.try_normalize() else { continue };
        if (n.dot(normal) + 1.0) * 0.5 <= me.facing {
            continue;
        }
        let count = cross.length() * 0.5 * per_area * fade + sim.mesh_carry;
        let k = count.round();
        sim.mesh_carry = count - k;
        let tangent = n.cross(e1).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
        let bitangent = tangent.cross(n);
        for _ in 0..k.max(0.0) as usize {
            if sim.particles.len() >= layer.max_particles as usize {
                if !overwrite {
                    return;
                }
                sim.particles.remove(0);
            }
            let (a, b) = (rng.next(), rng.next());
            let pos = v0 + e2 * a + e1 * (b * (1.0 - a));
            let azimuth = rng.next() - 0.5;
            let r = azimuth.abs().min(0.5 - azimuth.abs());
            let polar = s_angle * (c0 + r * c2) + s_angle * (c1 + r * c3) * rng.next();
            let (sa, ca) = (azimuth * TAU).sin_cos();
            let (sp_, cp) = (polar * TAU).sin_cos();
            let dir = bitangent * (sa * sp_) + tangent * (ca * sp_) + n * cp;
            let speed = rng.range(sp.speed) * s_speed;
            let local = layer.to_world().is_some();
            let (mut q, pos, vel) = birth(layer, secondaries, p, pos, dir * speed, local, rng);
            q.pos = pos;
            q.vel = vel;
            q.serial = sim.spawned;
            sim.spawned = sim.spawned.wrapping_add(1);
            sim.particles.push(q);
        }
    }
}

/// `Initialise3dRotation` (`FUN_00453070`).
pub(super) fn init_rotation(op: &Op, q: &mut Particle, p: &Params, rng: &mut Rng) {
    let &Op::Rotation3dInit { euler, local_spin, revs, random_spin, random_rotation, .. } = op else { return };
    let (_, frame, _) = p.transform.to_scale_rotation_translation();
    q.quat = if random_rotation {
        let v = Vec4::new(rng.next(), rng.next(), rng.next(), rng.next()) * 2.0 - Vec4::ONE;
        Quat::from_vec4(v.try_normalize().unwrap_or(Vec4::W))
    } else {
        // `qz ⊗ qy ⊗ qx` about the world axes after the emitter's own orientation.
        let e = Quat::from_rotation_z(euler[2]) * Quat::from_rotation_y(euler[1]) * Quat::from_rotation_x(euler[0]);
        e * frame
    };
    q.omega = if random_spin {
        let phi = TAU * rng.next();
        let z = 2.0 * rng.next() - 1.0;
        let s = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(phi.sin() * s, phi.cos() * s, z) * (revs * TAU)
    } else {
        p.transform.transform_vector3(Vec3::from(local_spin))
    };
}

/// `Update3dRotation` (`FUN_0045ddf0`): turn by the angular velocity, then damp it by `(1 − damping)^dt`.
pub(super) fn rotate(q: &mut Particle, damping: f32, dt: f32) {
    if q.omega.abs().max_element() <= f32::EPSILON {
        return;
    }
    let speed = q.omega.length();
    let d = Quat::from_axis_angle(q.omega / speed, speed * dt);
    q.quat = d * q.quat;
    q.omega *= (1.0 - damping).max(0.0).powf(dt);
}

/// `UpdatePosVelWithWorldCollision` (`FUN_00462750`; `Op::WorldCollide`). The wind term (`W`, engine block +0x50,
/// not identified) is left out; at most 40 particles test the world per update, as in the game.
pub(super) fn world_collide(op: &Op, particles: &mut [Particle], dt: f32, rng: &mut Rng, world: WorldCast) {
    let &Op::WorldCollide { keep, gravity, drag, turbulence, restitution, rest_speed, .. } = op else { return };
    let retain = (1.0 - drag).max(0.0).powf(dt);
    let mut queries = 0;
    for q in particles.iter_mut() {
        if q.cr < 0.0 {
            continue;
        }
        let jitter = Vec3::new(rng.next(), rng.next(), rng.next()) * 2.0 - Vec3::ONE;
        let delta = Vec3::new(0.0, gravity * dt, 0.0) + jitter * (turbulence * dt);
        let start = q.pos;
        let end_pos = q.pos + q.vel * dt;
        let ahead = q.vel.try_normalize().unwrap_or(Vec3::ZERO) * q.cr;
        let vel = q.vel * retain + delta;
        q.pos = end_pos;
        q.vel = vel;
        if queries >= 40 {
            continue;
        }
        queries += 1;
        let end = end_pos + ahead;
        let Some((t, n)) = world(start, end) else { continue };
        let seg = end - start;
        let d = seg.dot(n).min(-1e-4);
        q.pos = start + seg * (t + q.cr / d);
        let vc = vel - delta;
        let k = (1.0 - keep[1]) + rng.next() * (keep[1] - keep[0]);
        let vn = vc.dot(n).min(0.0);
        q.vel = (vc - n * vn) * k - n * (vn * restitution);
        if q.vel.length_squared() > rest_speed * rest_speed || n.y <= 0.9 {
            q.omega *= k;
        } else {
            q.vel = Vec3::ZERO;
            q.omega = Vec3::ZERO;
            q.cr = -q.cr;
        }
    }
}

/// `Render3dSprite` / `Render3dSpriteSubUV` (`FUN_00455670` / `FUN_00456110`): a card folded along its local Z, two
/// panels `r` wide and `2r` long whose outer edges rise 22.5° towards local +Y (`.rdata` 0x00f9d220 / 0x00f9d230),
/// turned by the particle's quaternion, its centre pushed `r` towards the camera. Lighting normals: the edges
/// `R·(∓sin 22.5°, −cos 22.5°, 0)` with wrap `−2k`, the spine `R·(0, cos 22.5°, 0)` with `+2k` (`k` = the
/// renderer's wrap word; the game's values, ported as they are).
pub(super) fn push_card(out: &mut Vec<super::QuadVertex>, d: &super::Drawn, r: &mw_formats::bear::Renderer, eye: Vec3) {
    const C: f32 = 0.923_879_5;
    const S: f32 = 0.382_683_43;
    let centre = d.pos + (eye - d.pos).normalize_or_zero() * d.radius;
    let (x, y, z) = (d.quat * Vec3::X, d.quat * Vec3::Y, d.quat * Vec3::Z);
    let corner = |a: f32, b: f32, c: f32| centre + (x * a + y * b + z * c) * d.radius;
    let k = r.lighting[2];
    let left = (x * -S + y * -C).normalize_or_zero();
    let right = (x * S + y * -C).normalize_or_zero();
    let spine = y * C;
    let frame = Vec2::new(d.subuv, if r.soft { r.depth_fade.max(0.0) } else { 0.0 });
    let light = |wrap: f32| Vec4::new(r.lighting[0], r.lighting[1], wrap, r.opacity);
    let verts = [
        (corner(-C, S, -1.0), Vec2::new(0.0, 0.0), left, -2.0 * k),
        (corner(-C, S, 1.0), Vec2::new(0.0, 1.0), left, -2.0 * k),
        (corner(0.0, 0.0, 1.0), Vec2::new(0.5, 1.0), spine, 2.0 * k),
        (corner(0.0, 0.0, -1.0), Vec2::new(0.5, 0.0), spine, 2.0 * k),
        (corner(0.0, 0.0, -1.0), Vec2::new(0.5, 0.0), spine, 2.0 * k),
        (corner(0.0, 0.0, 1.0), Vec2::new(0.5, 1.0), spine, 2.0 * k),
        (corner(C, S, 1.0), Vec2::new(1.0, 1.0), right, -2.0 * k),
        (corner(C, S, -1.0), Vec2::new(1.0, 0.0), right, -2.0 * k),
    ];
    for (pos, uv, normal, wrap) in verts {
        out.push(super::QuadVertex { pos, normal, uv, frame, light: light(wrap), colour: d.colour });
    }
}

// ---------------------------------------------------------------------------------------------------
// Mesh debris (`RenderMesh` 0x22 / `RenderMeshNoPaintColour` 0x23, run `FUN_00457720`)

/// One vertex of a debris piece (model space).
#[derive(Clone, Copy)]
struct DebrisVertex {
    pos: Vec3,
    normal: Vec3,
    uv: Vec2,
}

/// A debris Model: one renderable whose triangles hold 16 pieces, told apart by `BLENDINDICES` (the vertex program
/// indexes `gaTransforms[3·index]`); its diffuse texture (RGBA8) and whether its shader is `VfxMeshCarPaint`.
struct DebrisModel {
    pieces: Vec<Vec<DebrisVertex>>,
    texture: Option<(u32, u32, Vec<u8>)>,
    car_paint: bool,
    alpha_test: bool,
}

/// One draw batch: a model with one paint colour.
struct DebrisBatch {
    entity: Entity,
    mesh: Handle<Mesh>,
    shown: bool,
}

type DebrisArrival = (mw_bundle::ResourceId, Option<DebrisModel>);

/// The debris models (loaded on first use on a worker thread) and their batches.
#[derive(Resource, Default)]
pub(crate) struct DebrisModels {
    models: HashMap<mw_bundle::ResourceId, Option<Arc<DebrisModel>>>,
    pending: Option<std::sync::Mutex<std::sync::mpsc::Receiver<DebrisArrival>>>,
    want: Vec<mw_bundle::ResourceId>,
    batches: HashMap<(mw_bundle::ResourceId, u32), DebrisBatch>,
}

fn load_debris(store: &mw_assets::AssetStore, id: mw_bundle::ResourceId) -> Option<DebrisModel> {
    let (meshes, errors) = store.model_meshes(id, 0).ok()?;
    for e in errors {
        warn!("debris model {id}: {e}");
    }
    let mut pieces = vec![Vec::new(); 16];
    let mut texture = None;
    let mut car_paint = false;
    let mut alpha_test = false;
    for sm in &meshes {
        let m = &sm.mesh;
        let uv = m.texcoord(0);
        for &i in &m.indices {
            let i = i as usize;
            let piece = m.joints.as_ref().and_then(|j| j.get(i)).map_or(0, |j| j[0] as usize & 15);
            let p = m.positions[i];
            let n = m.normals.as_ref().and_then(|n| n.get(i)).map_or([0.0, 1.0, 0.0, 0.0], |n| *n);
            let t = uv.and_then(|u| u.get(i)).map_or([0.0; 4], |u| *u);
            pieces[piece].push(DebrisVertex {
                pos: Vec3::new(p[0], p[1], p[2]),
                normal: Vec3::new(n[0], n[1], n[2]),
                uv: Vec2::new(t[0], t[1]),
            });
        }
        if texture.is_none() {
            texture = sm.diffuse.and_then(|t| store.texture(t).ok()).and_then(|t| t.decode_rgba8(0));
            car_paint = sm.shader_name.contains("CarPaint");
            alpha_test = sm.shader_name == "VfxMesh";
        }
    }
    let extent = pieces.iter().flatten().map(|v| v.pos.abs().max_element()).fold(0.0, f32::max);
    let triangles = pieces.iter().map(Vec::len).sum::<usize>() / 3;
    info!("debris model {id}: {triangles} triangles, extent {extent:.2}, car paint {car_paint}");
    Some(DebrisModel { pieces, texture, car_paint, alpha_test })
}

/// The 8-bit sRGB value of a linear one.
fn to_srgb(x: f32) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let s = if x <= 0.003_130_8 { x * 12.92 } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

fn to_linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

/// A batch's material: the diffuse texture; for `VfxMeshCarPaint` baked with the paint colour as its pixel program
/// does it (`albedo = tex.rgb + g_paintColour.rgb · (1 − tex.a)`).
fn debris_material(model: &DebrisModel, paint: Option<Vec3>, images: &mut Assets<Image>) -> StandardMaterial {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let image = model.texture.as_ref().map(|(w, h, rgba)| {
        let mut data = rgba.clone();
        if let Some(paint) = paint {
            for px in data.as_chunks_mut::<4>().0 {
                let a = px[3] as f32 / 255.0;
                for k in 0..3 {
                    px[k] = to_srgb(to_linear(px[k]) + paint[k] * (1.0 - a));
                }
                px[3] = 255;
            }
        }
        let mut img = Image::new(
            Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.sampler = bevy::image::ImageSampler::linear();
        images.add(img)
    });
    StandardMaterial {
        base_color_texture: image,
        perceptual_roughness: 0.6,
        alpha_mode: if model.alpha_test && paint.is_none() { AlphaMode::Mask(0.5) } else { AlphaMode::Opaque },
        ..default()
    }
}

/// Load the debris models the live effects draw and rebuild their batches: each live particle of a mesh layer
/// draws piece `serial & 15` of the model (the game's 16 instance slots per bucket) with the matrix `R(q) · radius`
/// at its position; `RenderMesh` (0x22) adds the car's paint colour (`VehicleParams` +0x10), 0x23 does not.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_debris(
    mut commands: Commands,
    game: Res<crate::Game>,
    instances: Res<super::Instances>,
    view: Res<super::BearView>,
    camera: Query<&GlobalTransform, crate::MainCamera>,
    mut debris: ResMut<DebrisModels>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut visibility: Query<&mut Visibility>,
) {
    let debris = &mut *debris;
    // Arrivals.
    if let Some(rx) = debris.pending.take() {
        let mut got = Vec::new();
        let done = loop {
            match rx.lock().unwrap().try_recv() {
                Ok(m) => got.push(m),
                Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        for (id, m) in got {
            debris.models.insert(id, m.map(Arc::new));
        }
        if !done {
            debris.pending = Some(rx);
        }
    }
    let Some(cam) = camera.iter().next() else { return };
    let view = super::BearView { eye: cam.translation(), forward: cam.forward().as_vec3(), ..*view };
    type Batched = (Option<Vec3>, Vec<DebrisVertex>);
    let mut verts: HashMap<(mw_bundle::ResourceId, u32), Batched> = HashMap::new();
    for inst in &instances.list {
        for (layer, sim) in inst.def.layers.iter().zip(&inst.layers) {
            let Some(r) = layer.renderer().filter(|r| matches!(r.kind, 0x22 | 0x23)) else { continue };
            let Some(model_id) = r.material else { continue };
            let model = match debris.models.get(&model_id) {
                Some(Some(m)) => m.clone(),
                Some(None) => continue,
                None => {
                    debris.models.insert(model_id, None);
                    debris.want.push(model_id);
                    continue;
                }
            };
            let paint = (r.kind == 0x22 && model.car_paint).then_some(inst.params.paint);
            // The batch key: the model and, for painted debris, its colour (8 bits a channel).
            let key = paint.map_or(0, |c| {
                let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0) as u32;
                1 << 24 | q(c.x) << 16 | q(c.y) << 8 | q(c.z)
            });
            let out = &mut verts.entry((model_id, key)).or_insert_with(|| (paint, Vec::new())).1;
            for q in &sim.particles {
                let Some(d) = super::render_particle(layer, q, &view, &sim.slots) else { continue };
                let piece = &model.pieces[q.serial as usize & 15];
                let rot = q.quat.normalize();
                for v in piece {
                    out.push(DebrisVertex { pos: d.pos + rot * (v.pos * d.radius), normal: rot * v.normal, uv: v.uv });
                }
            }
        }
    }
    // Requests.
    if debris.pending.is_none() && !debris.want.is_empty() {
        let want = std::mem::take(&mut debris.want);
        let store = game.store.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        crate::platform::thread::Builder::new()
            .name("debris models".into())
            .spawn(move || {
                for id in want {
                    let _ = tx.send((id, load_debris(&store, id)));
                }
            })
            .expect("spawn debris loader");
        debris.pending = Some(std::sync::Mutex::new(rx));
    }
    // Batches with nothing to draw this frame are hidden (an empty mesh upsets Bevy's mesh allocator).
    for (key, b) in debris.batches.iter_mut() {
        if b.shown && verts.get(key).is_none_or(|v| v.1.is_empty()) {
            b.shown = false;
            if let Ok(mut v) = visibility.get_mut(b.entity) {
                *v = Visibility::Hidden;
            }
        }
    }
    for (key, (paint, v)) in verts {
        if v.is_empty() {
            continue;
        }
        let Some(Some(model)) = debris.models.get(&key.0) else { continue };
        let b = debris.batches.entry(key).or_insert_with(|| {
            let mesh = meshes.add(debris_mesh(&v));
            let material = materials.add(debris_material(model, paint, &mut images));
            let entity = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    Visibility::Visible,
                    bevy::camera::visibility::NoFrustumCulling,
                ))
                .id();
            DebrisBatch { entity, mesh, shown: true }
        });
        let _ = meshes.insert(&b.mesh, debris_mesh(&v));
        if !b.shown {
            b.shown = true;
            if let Ok(mut vis) = visibility.get_mut(b.entity) {
                *vis = Visibility::Visible;
            }
        }
    }
}

fn debris_mesh(v: &[DebrisVertex]) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, v.iter().map(|v| v.pos).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, v.iter().map(|v| v.normal).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, v.iter().map(|v| v.uv).collect::<Vec<_>>())
}
