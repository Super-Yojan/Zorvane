//! One rover's side of the FPV call, as TerraPhone does it: answer a
//! dashboard `start` with an H.264 offer and stream the rover's camera.
//! Zenoh carries only effects from `SignalingSession::phone`. Ported from ARGOS
//! `argos-gpui/src/fpv/sender.rs`, with the camera feed in place of its test
//! pattern.

use super::link::Bus;
use super::peer::{build_peer, preferred_video_codecs};
use super::{FPS, Feed, HEIGHT, WIDTH};
use bytes::Bytes;
use fpv_signaling::{CallState, Effect, SignalingSession, Topic};
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile};
use openh264::formats::{RgbSliceU8, YUVBuffer};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use webrtc::api::media_engine::MIME_TYPE_H264;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

pub(super) struct SenderOptions {
    pub endpoint: String,
    pub prefix: String,
    pub rover: u64,
    pub user: Option<String>,
    pub password: Option<String>,
    pub stun: Vec<String>,
    pub feed: Arc<Feed>,
    pub cancel: Arc<AtomicBool>,
}

pub(super) async fn run_sender(options: SenderOptions) {
    if let Err(error) = serve(&options).await {
        eprintln!("Zorvane FPV rover {}: {error}", options.rover);
    }
    options.feed.active.store(false, Ordering::Relaxed);
}

async fn serve(options: &SenderOptions) -> Result<(), String> {
    let credentials = options.user.as_deref().zip(options.password.as_deref());
    let mut bus = Bus::connect(&options.endpoint, &options.prefix, credentials)?;
    let mut requests = bus.subscribe(options.rover, Topic::Request);
    let mut answers = bus.subscribe(options.rover, Topic::Answer);
    let mut ice_in = bus.subscribe(options.rover, Topic::IceDashboard);
    let mut machine = SignalingSession::phone();
    let mut live: Option<Live> = None;
    let mut pending_ice = Vec::new();
    let mut session = String::new();
    let clock = Instant::now();
    while !options.cancel.load(Ordering::Relaxed) {
        let now = elapsed_ms(clock);
        let mut effects = Vec::new();
        for (receiver, topic) in [
            (&mut requests, Topic::Request),
            (&mut answers, Topic::Answer),
            (&mut ice_in, Topic::IceDashboard),
        ] {
            while let Ok(bytes) = receiver.try_recv() {
                if let Ok(text) = String::from_utf8(bytes) {
                    effects.extend(machine.receive(topic.suffix(), &text, now));
                }
            }
        }
        if let Some(current) = live.as_mut() {
            let local: Vec<_> = std::iter::from_fn(|| current.local_ice.try_recv().ok()).collect();
            let states: Vec<_> = std::iter::from_fn(|| current.states.try_recv().ok()).collect();
            for init in local {
                let (candidate, sdp_mid, index) = ice_parts(init);
                effects.extend(machine.note_local_ice(&session, &candidate, &sdp_mid, index, now));
            }
            for state in states {
                if let Some(call) = call_state(state) {
                    effects.extend(machine.note_media(&session, call, now));
                }
            }
        }
        effects.extend(machine.tick(now));
        apply(
            &mut machine,
            &bus,
            options,
            &mut live,
            &mut pending_ice,
            &mut session,
            now,
            effects,
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if machine.has_session() {
        let now = elapsed_ms(clock);
        let effects = machine.stop(true);
        apply(
            &mut machine,
            &bus,
            options,
            &mut live,
            &mut pending_ice,
            &mut session,
            now,
            effects,
        )
        .await?;
    }
    Ok(())
}

struct Live {
    peer: Arc<RTCPeerConnection>,
    track: Arc<TrackLocalStaticSample>,
    local_ice: std::sync::mpsc::Receiver<Option<RTCIceCandidateInit>>,
    states: std::sync::mpsc::Receiver<RTCPeerConnectionState>,
    generation: Arc<AtomicU64>,
    encoding: AtomicBool,
    remote_ready: bool,
}

#[allow(clippy::too_many_arguments)]
async fn apply(
    machine: &mut SignalingSession,
    bus: &Bus,
    options: &SenderOptions,
    live: &mut Option<Live>,
    pending_ice: &mut Vec<RTCIceCandidateInit>,
    session: &mut String,
    now: u64,
    effects: Vec<Effect>,
) -> Result<(), String> {
    let mut queue = VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        match effect {
            Effect::Publish { topic, payload } => bus.publish(options.rover, topic, &payload)?,
            Effect::CreateOffer { session: id } => {
                *session = id.clone();
                match open_offer(&options.stun).await {
                    Ok((created, sdp)) => {
                        *live = Some(created);
                        queue.extend(machine.note_local_description(&id, &sdp, now));
                    }
                    Err(error) => {
                        eprintln!("Zorvane FPV rover {}: offer failed: {error}", options.rover);
                        queue.extend(machine.note_media(&id, CallState::Failed, now));
                    }
                }
            }
            Effect::ApplyAnswer { sdp, .. } => {
                if let Some(current) = live.as_mut() {
                    let remote =
                        RTCSessionDescription::answer(sdp).map_err(|error| error.to_string())?;
                    current
                        .peer
                        .set_remote_description(remote)
                        .await
                        .map_err(|error| error.to_string())?;
                    current.remote_ready = true;
                    flush_ice(&current.peer, pending_ice).await;
                }
            }
            Effect::AddIce {
                candidate,
                sdp_mid,
                sdp_m_line_index,
                ..
            } => {
                pending_ice.push(candidate_init(candidate, sdp_mid, sdp_m_line_index));
                if let Some(current) = live.as_ref().filter(|current| current.remote_ready) {
                    flush_ice(&current.peer, pending_ice).await;
                }
            }
            Effect::EndOfCandidates { .. } | Effect::AcceptOffer { .. } => {}
            Effect::StopMedia { .. } => {
                pending_ice.clear();
                if let Some(current) = live.take() {
                    current.generation.fetch_add(1, Ordering::Relaxed);
                    let _ = current.peer.close().await;
                }
                options.feed.active.store(false, Ordering::Relaxed);
            }
            Effect::Show { state, detail, .. } => match state {
                CallState::Connected => {
                    if let Some(current) = live.as_ref() {
                        start_encoder(current, options);
                    }
                }
                CallState::Failed => {
                    eprintln!("Zorvane FPV rover {}: {detail}", options.rover);
                }
                _ => {}
            },
        }
    }
    Ok(())
}

async fn open_offer(stun: &[String]) -> Result<(Live, String), String> {
    let peer = build_peer(stun).await?;
    let track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                .into(),
            rtcp_feedback: vec![],
        },
        "fpv".into(),
        "zorvane-fpv".into(),
    ));
    peer.add_track(track.clone() as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(transceiver) = peer.get_transceivers().await.into_iter().next() {
        transceiver
            .set_codec_preferences(preferred_video_codecs())
            .await
            .map_err(|error| error.to_string())?;
        transceiver
            .set_direction(RTCRtpTransceiverDirection::Sendonly)
            .await;
    }
    let (local_ice_tx, local_ice) = std::sync::mpsc::sync_channel(64);
    peer.on_ice_candidate(Box::new(move |candidate| {
        let init = candidate.and_then(|candidate| candidate.to_json().ok());
        let _ = local_ice_tx.try_send(init);
        Box::pin(async {})
    }));
    let (state_tx, states) = std::sync::mpsc::sync_channel(8);
    peer.on_peer_connection_state_change(Box::new(move |state| {
        let _ = state_tx.try_send(state);
        Box::pin(async {})
    }));
    let offer = peer
        .create_offer(None)
        .await
        .map_err(|error| error.to_string())?;
    let sdp = offer.sdp.clone();
    peer.set_local_description(offer)
        .await
        .map_err(|error| error.to_string())?;
    Ok((
        Live {
            peer,
            track,
            local_ice,
            states,
            generation: Arc::new(AtomicU64::new(1)),
            encoding: AtomicBool::new(false),
            remote_ready: false,
        },
        sdp,
    ))
}

/// Turns the rover's FPV camera on and streams its frames until the call
/// ends. A frame is resent unchanged if the camera has not produced a new one.
fn start_encoder(active: &Live, options: &SenderOptions) {
    if active.encoding.swap(true, Ordering::Relaxed) {
        return;
    }
    let track = active.track.clone();
    let generation = active.generation.clone();
    let seen = generation.load(Ordering::Relaxed);
    let cancel = options.cancel.clone();
    let feed = options.feed.clone();
    let rover = options.rover;
    feed.active.store(true, Ordering::Relaxed);
    tokio::spawn(async move {
        let period = Duration::from_secs_f32(1.0 / FPS);
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(1_500_000))
            .max_frame_rate(FrameRate::from_hz(FPS))
            .skip_frames(false)
            .intra_frame_period(IntraFramePeriod::from_num_frames(FPS as u32))
            .profile(Profile::Baseline);
        let mut encoder =
            match Encoder::with_api_config(openh264::OpenH264API::from_source(), config) {
                Ok(encoder) => encoder,
                Err(error) => {
                    eprintln!("Zorvane FPV rover {rover}: encoder failed: {error}");
                    return;
                }
            };
        let mut rgb = vec![0u8; (WIDTH * HEIGHT * 3) as usize];
        let mut frame = 0u32;
        let mut next = Instant::now();
        while !cancel.load(Ordering::Relaxed) && generation.load(Ordering::Relaxed) == seen {
            let latest = feed.latest.lock().unwrap().clone();
            if let Some(rgba) = latest {
                for (out, pixel) in rgb
                    .as_chunks_mut::<3>()
                    .0
                    .iter_mut()
                    .zip(rgba.as_chunks::<4>().0)
                {
                    out.copy_from_slice(&pixel[..3]);
                }
                if frame.is_multiple_of(FPS as u32) {
                    encoder.force_intra_frame();
                }
                let yuv = YUVBuffer::from_rgb8_source(RgbSliceU8::new(
                    &rgb,
                    (WIDTH as usize, HEIGHT as usize),
                ));
                // The encoded bitstream borrows the encoder and is not Send.
                let data = match encoder.encode(&yuv) {
                    Ok(encoded) => encoded.to_vec(),
                    Err(error) => {
                        eprintln!("Zorvane FPV rover {rover}: encode failed: {error}");
                        break;
                    }
                };
                if !data.is_empty() {
                    let sample = Sample {
                        data: Bytes::from(data),
                        duration: period,
                        ..Default::default()
                    };
                    let _ = track.write_sample(&sample).await;
                }
                frame = frame.wrapping_add(1);
            }
            next += period;
            let now = Instant::now();
            if next > now {
                tokio::time::sleep(next - now).await;
            } else {
                next = now;
            }
        }
        feed.active.store(false, Ordering::Relaxed);
    });
}

fn call_state(state: RTCPeerConnectionState) -> Option<CallState> {
    match state {
        RTCPeerConnectionState::Connected => Some(CallState::Connected),
        RTCPeerConnectionState::Failed => Some(CallState::Failed),
        RTCPeerConnectionState::Closed => Some(CallState::Closed),
        RTCPeerConnectionState::Disconnected => Some(CallState::Connecting),
        _ => None,
    }
}

fn ice_parts(init: Option<RTCIceCandidateInit>) -> (String, String, i32) {
    match init {
        Some(init) => (
            init.candidate,
            init.sdp_mid.unwrap_or_default(),
            i32::from(init.sdp_mline_index.unwrap_or(0)),
        ),
        None => (String::new(), String::new(), 0),
    }
}

fn candidate_init(
    candidate: String,
    sdp_mid: String,
    sdp_m_line_index: i32,
) -> RTCIceCandidateInit {
    RTCIceCandidateInit {
        candidate,
        sdp_mid: (!sdp_mid.is_empty()).then_some(sdp_mid),
        sdp_mline_index: u16::try_from(sdp_m_line_index).ok(),
        username_fragment: None,
    }
}

async fn flush_ice(peer: &RTCPeerConnection, pending: &mut Vec<RTCIceCandidateInit>) {
    for init in pending.drain(..) {
        let _ = peer.add_ice_candidate(init).await;
    }
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}
