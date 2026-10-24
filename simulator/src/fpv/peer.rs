//! WebRTC peer and codec setup, matching TerraPhone's offer: H.264 profiles
//! first, then VP8. Ported from ARGOS `argos-gpui/src/fpv/session.rs`.

use std::sync::Arc;
use std::time::Duration;
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_H264, MIME_TYPE_VP8, MediaEngine};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::ice::mdns::MulticastDnsMode;
use webrtc::ice::network_type::NetworkType;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::rtp_transceiver::RTCPFeedback;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};

/// STUN servers from `ZORVANE_FPV_STUN` (comma-separated `stun:` URLs).
/// Unset means host candidates only, which is what a dashboard on the same
/// computer or LAN needs.
pub(super) fn stun_from_env() -> Result<Vec<String>, String> {
    let Ok(raw) = std::env::var("ZORVANE_FPV_STUN") else {
        return Ok(Vec::new());
    };
    let mut urls = Vec::new();
    for part in raw.split(',') {
        let url = part.trim();
        if url.is_empty() || url.eq_ignore_ascii_case("none") {
            continue;
        }
        if !(url.starts_with("stun:") || url.starts_with("stuns:")) {
            return Err("ZORVANE_FPV_STUN entries must start with stun: or stuns:".into());
        }
        urls.push(url.to_string());
    }
    Ok(urls)
}

pub(super) async fn build_peer(stun: &[String]) -> Result<Arc<RTCPeerConnection>, String> {
    let mut media = MediaEngine::default();
    for codec in preferred_video_codecs() {
        media
            .register_codec(codec, RTPCodecType::Video)
            .map_err(|error| error.to_string())?;
    }
    let registry = register_default_interceptors(Registry::new(), &mut media)
        .map_err(|error| error.to_string())?;
    let mut setting = SettingEngine::default();
    setting.detach_data_channels();
    setting.set_ice_multicast_dns_mode(MulticastDnsMode::Disabled);
    setting.set_network_types(vec![NetworkType::Udp4]);
    setting.set_host_acceptance_min_wait(Some(Duration::ZERO));
    let api = APIBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_setting_engine(setting)
        .build();
    let ice_servers = if stun.is_empty() {
        Vec::new()
    } else {
        vec![RTCIceServer {
            urls: stun.to_vec(),
            ..RTCIceServer::default()
        }]
    };
    Ok(Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers,
            ..RTCConfiguration::default()
        })
        .await
        .map_err(|error| error.to_string())?,
    ))
}

/// H.264 profiles first, then VP8. Matches the phone's offer order.
pub(super) fn preferred_video_codecs() -> Vec<RTCRtpCodecParameters> {
    let feedback = vec![
        RTCPFeedback {
            typ: "goog-remb".into(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "ccm".into(),
            parameter: "fir".into(),
        },
        RTCPFeedback {
            typ: "nack".into(),
            parameter: String::new(),
        },
        RTCPFeedback {
            typ: "nack".into(),
            parameter: "pli".into(),
        },
    ];
    let mut codecs: Vec<_> = [
        (
            102,
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
        ),
        (
            125,
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f",
        ),
        (
            108,
            "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f",
        ),
    ]
    .into_iter()
    .map(|(payload_type, fmtp)| RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: fmtp.to_owned(),
            rtcp_feedback: feedback.clone(),
        },
        payload_type,
        ..RTCRtpCodecParameters::default()
    })
    .collect();
    codecs.push(RTCRtpCodecParameters {
        capability: RTCRtpCodecCapability {
            mime_type: MIME_TYPE_VP8.to_owned(),
            clock_rate: 90_000,
            channels: 0,
            sdp_fmtp_line: String::new(),
            rtcp_feedback: feedback,
        },
        payload_type: 96,
        ..RTCRtpCodecParameters::default()
    });
    codecs
}
