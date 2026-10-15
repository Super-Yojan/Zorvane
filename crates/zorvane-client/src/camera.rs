//! Decode the simulator's existing newline-delimited header + binary pixel packet.

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct CameraHeader {
    pub rover_id: u64,
    pub width: u32,
    pub height: u32,
    pub sequence: u64,
    pub encoding: String,
    /// Includes intrinsics, exposure time, and optical/body poses when present.
    pub metadata: Value,
}

#[derive(Clone, Debug)]
pub struct CameraFrame {
    pub header: CameraHeader,
    pub pixels: Vec<u8>,
}

impl CameraFrame {
    pub fn depth_metres(&self) -> Option<Vec<f32>> {
        (self.header.encoding == "32FC1_LE").then(|| {
            self.pixels
                .chunks_exact(4)
                .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
                .collect()
        })
    }
}

pub fn decode_camera_frame(packet: &[u8]) -> Option<CameraFrame> {
    let split = packet.iter().take(65536).position(|&b| b == b'\n')?;
    let value: Value = serde_json::from_slice(&packet[..split]).ok()?;
    if value["version"].as_u64()? != 1 {
        return None;
    }
    let width = u32::try_from(value["width"].as_u64()?).ok()?;
    let height = u32::try_from(value["height"].as_u64()?).ok()?;
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return None;
    }
    let encoding = value["encoding"].as_str()?;
    if !matches!(encoding, "32FC1_LE" | "RGBA8_SRGB")
        || packet.len() - split - 1 != width as usize * height as usize * 4
    {
        return None;
    }
    Some(CameraFrame {
        header: CameraHeader {
            rover_id: value["rover_id"].as_u64()?,
            width,
            height,
            sequence: value["sequence"].as_u64()?,
            encoding: encoding.into(),
            metadata: value,
        },
        pixels: packet[split + 1..].to_vec(),
    })
}
