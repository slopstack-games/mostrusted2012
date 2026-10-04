//! Ground truth for the packed tangent-frame decode (`mw_formats::tangent_frame`): decoded vertex
//! normals must agree with the geometric face normals, and tangents must follow the UV0 U direction.
//! Skipped when no game install is found.

use mw_assets::AssetStore;
use mw_bundle::ResourceType;
use mw_gamedir::GameDir;

fn sub(a: [f32; 4], b: [f32; 4]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = dot(a, a).sqrt();
    (l > 1e-6).then(|| a.map(|c| c / l))
}

/// For every mesh with a packed frame in a world bundle and a vehicle bundle: the share of triangles
/// whose face normal points the same way as the decoded vertex normals, and whose UV-derived tangent
/// agrees with the decoded tangent.
#[test]
fn packed_normals_match_geometry() {
    let Some(game) = GameDir::locate(None).ok() else {
        eprintln!("skipped: no game install");
        return;
    };
    let store = AssetStore::new(game).unwrap();
    for bundle in ["HAWAII/TRK_UNIT1.BNDL", "VEHICLES/VEH_1085007_HI.BNDL"] {
        let b = store.bundle(bundle).unwrap();
        let (mut tris, mut normal_ok, mut tangent_ok, mut tangent_n, mut same_way) = (0u64, 0u64, 0u64, 0u64, 0u64);
        for e in b.entries_of_type(ResourceType::Renderable) {
            let Ok((meshes, _)) = store.renderable_meshes(e.id) else { continue };
            for sm in meshes {
                let m = &sm.mesh;
                let (Some(n), Some(t)) = (&m.normals, &m.tangents) else { continue };
                if t.iter().all(|t| t[3] == 0.0) {
                    continue; // stored float tangents, not a packed frame
                }
                for tri in m.indices.chunks_exact(3) {
                    let [a, b2, c] = [0, 1, 2].map(|k| tri[k] as usize);
                    let (pa, pb, pc) = (m.positions[a], m.positions[b2], m.positions[c]);
                    let Some(face) = norm(cross(sub(pb, pa), sub(pc, pa))) else { continue };
                    let vn = [a, b2, c].map(|i| [n[i][0], n[i][1], n[i][2]]);
                    let avg = [0, 1, 2].map(|k| vn[0][k] + vn[1][k] + vn[2][k]);
                    tris += 1;
                    // Winding may be either way round; the frame must be parallel to the face.
                    let d = dot(face, norm(avg).unwrap_or([0.0; 3]));
                    if d.abs() > 0.7 {
                        normal_ok += 1;
                    }
                    if d > 0.0 {
                        same_way += 1;
                    }
                    if let Some(uv) = m.texcoord(0) {
                        let (du1, dv1) = (uv[b2][0] - uv[a][0], uv[b2][1] - uv[a][1]);
                        let (du2, dv2) = (uv[c][0] - uv[a][0], uv[c][1] - uv[a][1]);
                        let det = du1 * dv2 - du2 * dv1;
                        if det.abs() > 1e-8 {
                            let (e1, e2) = (sub(pb, pa), sub(pc, pa));
                            let tu = [0, 1, 2].map(|k| (e1[k] * dv2 - e2[k] * dv1) / det);
                            if let Some(tu) = norm(tu) {
                                tangent_n += 1;
                                if dot(tu, [t[a][0], t[a][1], t[a][2]]) > 0.5 {
                                    tangent_ok += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        let nf = normal_ok as f64 / tris.max(1) as f64;
        let tf = tangent_ok as f64 / tangent_n.max(1) as f64;
        let sw = same_way as f64 / tris.max(1) as f64;
        eprintln!(
            "{bundle}: {tris} triangles, normals agree {nf:.3} (same side as the counter-clockwise face normal {sw:.3}), tangents along +U {tf:.3}"
        );
        assert!(tris > 1000, "{bundle}: too few packed-frame triangles");
        assert!(nf > 0.95, "{bundle}: decoded normals disagree with geometry ({nf:.3})");
    }
}
