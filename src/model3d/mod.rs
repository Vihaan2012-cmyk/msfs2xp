//! 3D models: Asobo GLB in, X-Plane OBJ8 out.

pub mod glb;
pub mod obj8;

pub use glb::{load_glb, AlphaMode, Material, Mesh, Model, ModelError, Vertex};
pub use obj8::{LodPart, ObjOptions, split_by_texture, write_obj8, write_obj8_lods};
