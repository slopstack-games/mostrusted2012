//! Parsers for NFS Most Wanted (2012) PC resource types.
//!
//! Each parser takes a decompressed [`mw_bundle::Resource`]. Pointers inside a
//! resource are offsets into its chunk 0; pointers to other resources are
//! resolved through the resource's import table.

pub mod ai_data;
mod animation;
pub mod bear;
pub mod binding;
pub mod camera_take;
mod collision;
mod colour_cube;
mod decode;
mod dxgi;
mod font;
pub mod genesys;
pub mod groundcover;
mod material;
mod paint_map;
mod renderable;
mod sampler;
mod text;
mod texture;
pub mod traffic;
pub mod trigger;
pub mod ui;
mod vehicle;
mod vertex;
mod world;
mod zone;

pub use ai_data::{AiData, AiJumpPoint, AiJunction, AiNode};
pub use animation::{Animation, AnimationList, Bone, BoneKey, Skeleton, decode_quat32};
pub use bear::BearEffect;
pub use collision::{CollisionTag, PolygonSoup, PolygonSoupList, SOUP_GRANULARITY, SOUP_OFFSET_UNIT, SoupPolygon};
pub use colour_cube::ColourCube;
pub use dxgi::{DxgiFormat, f16_to_f32};
pub use font::{Font, Glyph, GlyphQuad};
pub use groundcover::{GroundcoverCollection, GroundcoverInstance, GroundcoverInstances, GroundcoverLayer, Group};
pub use material::{Material, MaterialParameter, MaterialSampler, Model, Shader, ShaderParameter, ShaderSampler};
pub use paint_map::WorldPaintMap;
pub use renderable::{DecodedMesh, Mesh, Renderable, SNORM_POSITION_SCALE, Topology, tangent_frame};
pub use sampler::SamplerState;
pub use text::{Language, VehicleEntry, VehicleList};
pub use texture::{Dimension, Texture};
pub use traffic::{LanePolyline, TrafficHull, TrafficJunction, TrafficLaneData, TrafficLightGroup, TrafficSection};
pub use trigger::{Collectable, Trigger, TriggerData, TriggerShape};
pub use vehicle::{Locator, LocatorGroup, VehicleGraphicsSpec, Wheel, WheelPart};
pub use vertex::{Semantic, VertexAttribute, VertexDescriptor};
pub use world::{Instance, InstanceList, InstanceOverrides, ObjectDef, ObjectInstance, ObjectInstanceList};
pub use zone::{Zone, ZoneList};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Bundle(#[from] mw_bundle::Error),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("malformed resource: {0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, Error>;
