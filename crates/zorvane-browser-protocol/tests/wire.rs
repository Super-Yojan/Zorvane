use zorvane_browser_protocol::*;

#[test]
fn camera_publications_preserve_every_byte() {
    let payload = vec![0, 255, b'\n', 128];
    let packet = Publication::new(
        "terra/rover/7/camera/depth",
        "application/octet-stream",
        payload,
    );
    assert_eq!(
        Publication::decode(&packet.encode().unwrap()).unwrap(),
        packet
    );
}

#[test]
fn malformed_and_oversized_publications_are_rejected() {
    for data in [vec![], vec![255; 4], vec![0; 8]] {
        assert!(Publication::decode(&data).is_err());
    }
    assert!(
        Publication::new("../bad", "application/json", vec![])
            .encode()
            .is_err()
    );
    assert!(
        Publication::new(
            "terra/rover/0/pose",
            "application/json",
            vec![0; MAX_PACKET_BYTES]
        )
        .encode()
        .is_err()
    );
}

#[test]
fn subscriptions_match_whole_topic_segments() {
    assert!(matches_topic("terra/rover/*/pose", "terra/rover/12/pose"));
    assert!(!matches_topic(
        "terra/rover/*/pose",
        "terra/rover/12/camera/pose"
    ));
    assert!(matches_topic(
        "terra/rover/**",
        "terra/rover/12/camera/depth"
    ));
    assert!(matches_topic(
        "terra/**/status",
        "terra/rover/12/goal/status"
    ));
    assert!(!valid_topic("terra/rover//pose", false));
    assert!(!valid_topic("terra/rover/*", false));
}

#[test]
fn queue_replaces_frames_but_preserves_reliable_order_and_fails_closed() {
    let mut queue = Outbox::default();
    queue
        .push(Publication::new(
            "terra/rover/0/camera/rgb",
            "application/octet-stream",
            vec![1],
        ))
        .unwrap();
    queue
        .push(Publication::new(
            "terra/rover/0/camera/rgb",
            "application/octet-stream",
            vec![2],
        ))
        .unwrap();
    queue
        .push(Publication::new(
            "terra/rover/0/search/report",
            "application/json",
            vec![3],
        ))
        .unwrap();
    assert_eq!(queue.pop().unwrap().payload, vec![3]);
    assert_eq!(queue.pop().unwrap().payload, vec![2]);
    assert!(queue.pop().is_none());
    for _ in 0..MAX_RELIABLE {
        queue
            .push(Publication::new(
                "terra/rover/0/cmd_vel",
                "application/json",
                vec![0],
            ))
            .unwrap();
    }
    assert!(
        queue
            .push(Publication::new(
                "terra/rover/0/cmd_vel",
                "application/json",
                vec![0]
            ))
            .is_err()
    );
}

#[test]
fn emergency_stop_survives_control_queue_pressure_and_runs_last() {
    use std::collections::VecDeque;
    let mut queue = VecDeque::new();
    enqueue_control_action(
        &mut queue,
        "safety".into(),
        br#"{"action":"stop","token":"s1"}"#.to_vec(),
        0,
    );
    for i in 1..300 {
        enqueue_control_action(&mut queue, "teleop".into(), b"{}".to_vec(), i);
    }
    let actions = drain_control_actions(&mut queue);
    assert_eq!(actions.len(), 256);
    assert_eq!(actions.last().unwrap().0, "safety");
    assert!(queue.is_empty());
}

#[test]
fn hardware_disarm_survives_control_queue_pressure_and_runs_last() {
    use std::collections::VecDeque;
    let mut queue = VecDeque::new();
    enqueue_control_action(
        &mut queue,
        "hardware".into(),
        br#"{"action":"disarm","token":"h1"}"#.to_vec(),
        0,
    );
    for i in 1..300 {
        enqueue_control_action(&mut queue, "teleop".into(), b"{}".to_vec(), i);
    }
    let actions = drain_control_actions(&mut queue);
    assert_eq!(actions.len(), 256);
    assert_eq!(actions.last().unwrap().0, "hardware");
    assert!(queue.is_empty());
}

#[test]
fn a_new_occupancy_baseline_removes_an_old_pending_heartbeat() {
    let mut queue = Outbox::default();
    queue
        .push(Publication::new(
            "terra/rover/0/map/occupancy/heartbeat",
            "application/json",
            br#"{"sequence":1}"#.to_vec(),
        ))
        .unwrap();
    queue
        .push(Publication::new(
            "terra/rover/0/map/occupancy",
            "application/json",
            br#"{"sequence":2}"#.to_vec(),
        ))
        .unwrap();
    assert_eq!(queue.pop().unwrap().topic, "terra/rover/0/map/occupancy");
    assert!(queue.pop().is_none());
}
