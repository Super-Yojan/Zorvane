use zorvane_client::decode_camera_frame;

#[test]
fn decodes_depth_without_changing_nan_or_exposure_metadata() {
    let mut packet = br#"{"version":1,"rover_id":3,"width":2,"height":1,"sequence":7,"encoding":"32FC1_LE","exposure_time":1.25,"body":{"x":2,"y":3,"yaw":0.4}}"#.to_vec();
    packet.push(b'\n');
    packet.extend_from_slice(&1.5f32.to_le_bytes());
    packet.extend_from_slice(&f32::NAN.to_le_bytes());
    let frame = decode_camera_frame(&packet).unwrap();
    let depth = frame.depth_metres().unwrap();
    assert_eq!(depth[0], 1.5);
    assert!(depth[1].is_nan());
    assert_eq!(frame.header.metadata["exposure_time"], 1.25);
    packet.pop();
    assert!(decode_camera_frame(&packet).is_none());
}

#[test]
fn rgb_bytes_are_not_decoded_as_depth() {
    let mut packet =
        br#"{"version":1,"rover_id":0,"width":1,"height":1,"sequence":1,"encoding":"RGBA8_SRGB"}"#
            .to_vec();
    packet.extend_from_slice(b"\n\x00\xff\x80\xff");
    let frame = decode_camera_frame(&packet).unwrap();
    assert_eq!(frame.pixels, [0, 255, 128, 255]);
    assert!(frame.depth_metres().is_none());
}
