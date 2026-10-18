//! Minecraft player, cosmetic and emote renderer core, independent of any graphics API.
//! Hosts drive a `Scene` and draw the ordered vertex buffers it returns.

pub mod animation;
pub mod bedrock;
pub mod build;
pub mod bundle;
pub mod cosmetic;
pub mod ir;
pub mod java;
pub mod math;
pub mod mesh;
pub mod molang;
pub mod player;
pub mod pose;
pub mod scene;

pub use build::Camera;
pub use cosmetic::BodySlot;
pub use ir::Model;
pub use math::Affine;
pub use player::Arms;
pub use scene::Scene;
