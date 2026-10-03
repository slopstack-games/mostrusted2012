//! Parsers for NFS Most Wanted (2012) PC resource types.
//!
//! Each parser takes a decompressed [`mw_bundle::Resource`]. Pointers inside a
//! resource are offsets into its chunk 0; pointers to other resources are
//! resolved through the resource's import table.

mod collision;
mod decode;
mod dxgi;
pub mod genesys;
mod material;
mod renderable;
mod text;
mod texture;
mod vehicle;
mod vertex;
mod world;

pub use collision::{PolygonSoup, PolygonSoupList, SOUP_GRANULARITY, SOUP_OFFSET_UNIT, SoupPolygon};
pub use dxgi::{DxgiFormat, f16_to_f32};
pub use material::{Material, MaterialParameter, MaterialSampler, Model, Shader, ShaderParameter, ShaderSampler};
pub use renderable::{DecodedMesh, Mesh, Renderable, SNORM_POSITION_SCALE, Topology};
pub use text::{Language, VehicleEntry, VehicleList};
pub use texture::{Dimension, Texture};
pub use vehicle::{VehicleGraphicsSpec, Wheel, WheelPart};
pub use vertex::{Semantic, VertexAttribute, VertexDescriptor};
pub use world::{Instance, InstanceList};

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
