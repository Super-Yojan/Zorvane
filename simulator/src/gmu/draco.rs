use bevy_3d_tiles_prepare::{DecodeError, DracoMesh};

pub fn decode(bytes: &[u8], ids: &[u32]) -> Result<DracoMesh, DecodeError> {
    use draco_core::{DecodeLimits, DecoderBuffer, Mesh, MeshDecoder};
    let mut limits = DecodeLimits::default();
    limits.max_points = 1_000_000;
    limits.max_faces = 2_000_000;
    limits.max_decoded_bytes = 128 * 1024 * 1024;
    let mut buffer = DecoderBuffer::new(bytes).with_limits(limits);
    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut buffer, &mut mesh)
        .map_err(|e| DecodeError::draco(e.to_string()))?;
    let mut attributes = Vec::with_capacity(ids.len());
    for &id in ids {
        let attribute = mesh
            .attribute_by_unique_id(id)
            .ok_or_else(|| DecodeError::draco(format!("missing attribute {id}")))?;
        let components = usize::from(attribute.num_components());
        attributes.push((
            id,
            components,
            attribute.read_f32s(mesh.num_points(), components),
        ));
    }
    let indices = mesh
        .faces()
        .iter()
        .flat_map(|face| face.iter().map(|point| point.0))
        .collect();
    Ok(DracoMesh {
        indices,
        attributes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use draco_core::{
        DataType, EncoderBuffer, EncoderOptions, FaceIndex, GeometryAttributeType, Mesh,
        MeshEncoder, PointAttribute, PointIndex,
    };

    fn triangle() -> Vec<u8> {
        let mut mesh = Mesh::new();
        mesh.set_num_points(3);
        let mut pos = PointAttribute::new();
        pos.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            3,
        );
        pos.buffer_mut()
            .update_f32s_le(0, &[0., 0., 0., 1., 0., 0., 0., 1., 0.]);
        pos.set_unique_id(7);
        mesh.add_attribute_preserve_unique_id(pos);
        let mut uv = PointAttribute::new();
        uv.init(
            GeometryAttributeType::TexCoord,
            2,
            DataType::Float32,
            false,
            3,
        );
        uv.buffer_mut().update_f32s_le(0, &[0., 0., 1., 0., 0., 1.]);
        uv.set_unique_id(13);
        mesh.add_attribute_preserve_unique_id(uv);
        mesh.set_num_faces(1);
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
        let mut encoder = MeshEncoder::new();
        encoder.set_mesh(mesh);
        let mut output = EncoderBuffer::new();
        encoder.encode(&EncoderOptions::new(), &mut output).unwrap();
        output.data().to_vec()
    }

    #[test]
    fn native_decode_preserves_attribute_ids_and_textured_triangle() {
        let decoded = decode(&triangle(), &[13, 7]).unwrap();
        assert_eq!(decoded.indices.len(), 3);
        let (_, components, uv) = &decoded.attributes[0];
        assert_eq!((decoded.attributes[0].0, *components), (13, 2));
        let (_, components, pos) = &decoded.attributes[1];
        assert_eq!((decoded.attributes[1].0, *components), (7, 3));
        let mut corners = Vec::new();
        for index in decoded.indices {
            let i = index as usize;
            assert_eq!(&uv[i * 2..i * 2 + 2], &pos[i * 3..i * 3 + 2]);
            corners.push([pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]]);
        }
        for corner in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
            assert!(corners.contains(&corner));
        }
    }

    #[test]
    fn rejects_corrupt_mesh_and_missing_attribute() {
        assert!(decode(b"not draco", &[7]).is_err());
        assert!(decode(&triangle(), &[999]).is_err());
    }

    #[test]
    fn compressed_glb_reaches_tile_preparation_without_browser_decoder() {
        use bevy_3d_tiles_prepare::{
            assemble_glb, draco_requests, prepare_tile_extracting_with_draco,
        };
        let compressed = triangle();
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_draco_mesh_compression"],
            "extensionsRequired": ["KHR_draco_mesh_compression"],
            "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1},
                "mode": 4, "extensions": {"KHR_draco_mesh_compression": {
                    "bufferView": 0, "attributes": {"POSITION": 7, "TEXCOORD_0": 13}
                }}
            }]}],
            "accessors": [
                {"componentType": 5126, "count": 3, "type": "VEC3", "min": [0,0,0], "max": [1,1,0]},
                {"componentType": 5126, "count": 3, "type": "VEC2"}
            ],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": compressed.len()}],
            "buffers": [{"byteLength": compressed.len()}]
        });
        let glb = assemble_glb(&serde_json::to_vec(&json).unwrap(), &compressed);
        let requests = draco_requests(&glb).unwrap();
        assert_eq!(requests.len(), 1);
        let decoded = requests
            .iter()
            .map(|r| decode(&r.compressed, &r.attr_ids).unwrap())
            .collect();
        let prepared = prepare_tile_extracting_with_draco(&glb, false, decoded)
            .unwrap()
            .unwrap();
        assert!(
            prepared.meshes.is_some(),
            "must not fall back to the unsupported browser decoder"
        );
    }
}
