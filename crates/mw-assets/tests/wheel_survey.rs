//! Diagnostic: per car, the VehicleGraphicsSpec wheel positions against the body mesh's extent, to find
//! cars whose wheels don't sit in their arches. `cargo test --release -p mw-assets --test wheel_survey --
//! --ignored --nocapture`.

use mw_assets::AssetStore;
use mw_bundle::ResourceType;
use mw_gamedir::GameDir;

#[test]
#[ignore]
fn wheel_survey() {
    let Some(game) = GameDir::locate(None).ok() else { return };
    let store = AssetStore::new(game).unwrap();
    let lang = store.language(mw_assets::DEFAULT_LANGUAGE).unwrap();
    for v in store.vehicles(&lang).unwrap() {
        let Some(bundle) = v.hi_bundle else { continue };
        let Some(b) = store.bundle(&bundle) else { continue };
        let Some(e) = b.entries_of_type(ResourceType::VehicleGraphicsSpec).next() else { continue };
        let spec = mw_formats::VehicleGraphicsSpec::parse(&b.load(e).unwrap()).unwrap();
        let Some(Some(body)) = spec.parts.first() else { continue };
        let model = store.model(*body).unwrap();
        let Some(Some(rid)) = model.renderables.first() else { continue };
        let (meshes, _) = store.renderable_meshes(*rid).unwrap();
        let (mut zmin, mut zmax) = (f32::MAX, f32::MIN);
        for m in &meshes {
            for p in &m.mesh.positions {
                zmin = zmin.min(p[2]);
                zmax = zmax.max(p[2]);
            }
        }
        let front: Vec<f32> = spec.wheels.iter().filter(|w| w.is_front()).map(|w| w.position[2]).collect();
        let rear: Vec<f32> = spec.wheels.iter().filter(|w| !w.is_front()).map(|w| w.position[2]).collect();
        let (f, r) = (front.first().copied().unwrap_or(0.0), rear.first().copied().unwrap_or(0.0));
        println!(
            "{:<40} body z {:6.2}..{:5.2}  front {:6.2} (overhang {:5.2})  rear {:6.2} (overhang {:5.2})  wheelbase {:5.2}",
            v.name,
            zmin,
            zmax,
            f,
            zmax - f,
            r,
            r - zmin,
            f - r
        );
    }
}
