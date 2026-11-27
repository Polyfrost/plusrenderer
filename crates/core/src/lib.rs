//! Minecraft player, cosmetic and emote renderer core, independent of any graphics API.
//! Hosts drive a `Scene` and draw the ordered vertex buffers it returns.

pub mod animation;
pub mod asset;
pub mod attach;
pub mod bedrock;
pub mod build;
pub mod bundle;
pub mod container;
pub mod controller;
pub mod cost;
pub mod cosmetic;
pub mod effects;
pub mod export;
pub mod ir;
pub mod issues;
pub mod java;
pub mod limits;
pub mod math;
pub mod mesh;
pub mod molang;
pub mod nameplate;
pub mod palette;
pub mod pet;
pub mod player;
pub mod pose;
pub mod reactions;
pub mod roles;
pub mod scene;
pub mod signals;
pub mod springs;
pub mod texture;
pub mod wearer;

pub use build::Camera;
pub use cosmetic::BodySlot;
pub use ir::Model;
pub use math::Affine;
pub use player::{Arms, ElytraPose};
pub use scene::Scene;

#[cfg(test)]
mod test_alloc;
