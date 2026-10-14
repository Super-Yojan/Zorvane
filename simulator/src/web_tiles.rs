//! Elevation sampling for the browser build.
//!
//! The desktop build asks bevytiles to fetch tiles. That crate pulls ureq and
//! rustls, which do not compile for wasm32, and it decodes on worker threads.
//! This module is the same metre math for the one Terrarium PNG we embed.

use bevy::prelude::*;
use image::RgbImage;
use std::collections::HashMap;

pub const MIN_ZOOM: u8 = 9;
const EQUATOR_CIRCUMFERENCE_M: f64 = 40_075_016.686;

#[derive(Clone)]
pub struct WorldConfig {
    pub anchor_x: i32,
    pub anchor_z: i32,
    pub base_zoom: u8,
    pub max_zoom: u8,
    pub tile_size: f32,
    pub origin_offset: Vec3,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            anchor_x: 306,
            anchor_z: 207,
            base_zoom: MIN_ZOOM,
            max_zoom: 15,
            tile_size: 66_400.0,
            origin_offset: Vec3::ZERO,
        }
    }
}

impl WorldConfig {
    /// Same web-mercator anchor as `bevytiles::config::WorldConfig::from_lat_lon`.
    pub fn from_lat_lon(lat: f64, lon: f64) -> Self {
        let n = 2f64.powi(i32::from(MIN_ZOOM));
        let lat_rad = lat.to_radians();
        let x = (lon + 180.0) / 360.0 * n;
        let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
        let anchor_x = x.floor() as i32;
        let anchor_z = y.floor() as i32;
        let tile_size = EQUATOR_CIRCUMFERENCE_M * lat_rad.cos() / n;
        Self {
            anchor_x,
            anchor_z,
            tile_size: tile_size as f32,
            origin_offset: Vec3::new(
                ((x - anchor_x as f64) * tile_size) as f32,
                0.0,
                ((y - anchor_z as f64) * tile_size) as f32,
            ),
            ..Default::default()
        }
    }
}

pub struct TerrainAnchor {
    pub world_offset: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub zoom: u8,
    pub x: i32,
    pub z: i32,
}

#[derive(Clone)]
pub struct HeightGrid {
    pub w: u32,
    pub h: u32,
    pub samples: Vec<u16>,
}

impl HeightGrid {
    pub fn from_terrarium(img: &RgbImage) -> Self {
        let samples = img
            .pixels()
            .map(|pixel| {
                let height =
                    f64::from(pixel[0]) * 256.0 + f64::from(pixel[1]) + f64::from(pixel[2]) / 256.0
                        - 32768.0;
                (height.round() + 32768.0).clamp(0.0, 65535.0) as u16
            })
            .collect();
        Self {
            w: img.width(),
            h: img.height(),
            samples,
        }
    }

    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let sample = |x: i32, y: i32| -> f32 {
            let x = x.clamp(0, self.w as i32 - 1) as u32;
            let y = y.clamp(0, self.h as i32 - 1) as u32;
            f32::from(self.samples[(y * self.w + x) as usize])
        };
        let fx = u * self.w as f32 - 0.5;
        let fy = v * self.h as f32 - 0.5;
        let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let top = sample(x0, y0) * (1.0 - tx) + sample(x0 + 1, y0) * tx;
        let bottom = sample(x0, y0 + 1) * (1.0 - tx) + sample(x0 + 1, y0 + 1) * tx;
        (top * (1.0 - ty) + bottom * ty) - 32768.0
    }
}

#[derive(Default)]
pub struct HeightGrids(pub HashMap<TileKey, HeightGrid>);

pub fn ground_height(
    grids: &HeightGrids,
    world: &WorldConfig,
    anchor: &TerrainAnchor,
    pos: Vec3,
) -> Option<f32> {
    let abs = pos - anchor.world_offset;
    for zoom in (world.base_zoom..=world.max_zoom).rev() {
        let size = f64::from(world.tile_size) / f64::from(1u32 << (zoom - world.base_zoom));
        let tx = (f64::from(abs.x) / size).floor() as i32;
        let tz = (f64::from(abs.z) / size).floor() as i32;
        if let Some(grid) = grids.0.get(&TileKey { zoom, x: tx, z: tz }) {
            let u = ((f64::from(abs.x) - tx as f64 * size) / size) as f32;
            let v = ((f64::from(abs.z) - tz as f64 * size) / size) as f32;
            return Some(grid.sample(u, v));
        }
    }
    None
}
