---
layout: ../layouts/DocPage.astro
title: FPV video
description: Answer the rover's WebRTC call and receive H.264 video, the same way ARGOS does on a real rover.
---

Each rover streams video from a camera on its front sensor mount: H.264 Baseline, 640 × 480, 15 frames a second. A real Terra rover does the same from its phone. Signaling is Terra's `fpv-signaling` contract on six keys under `terra/rover/<id>/webrtc/`, so a dashboard that answers the simulator answers a rover.

The dashboard asks; the rover offers; the dashboard answers.

| Step | Key | Body |
| --- | --- | --- |
| 1 | `webrtc/request` | `{"v":1,"session":"<uuid>","action":"start"}` |
| 2 | `webrtc/offer` | `{"v":1,"session":"<uuid>","sdp":"…"}` |
| 2 | `webrtc/ice/phone` | `{"v":1,"session":"<uuid>","candidate":"…","sdpMid":"0","sdpMLineIndex":0}` |
| 3 | `webrtc/answer` | `{"v":1,"session":"<uuid>","sdp":"…"}` |
| 3 | `webrtc/ice/dashboard` | Your candidates, then one with `"candidate":""` to end gathering |
| 4 | `webrtc/state` | `{"v":1,"session":"<uuid>","state":"connected"}`, or `connecting`, `failed`, `closed` |
| 5 | `webrtc/request` | `{"v":1,"session":"<uuid>","action":"stop"}` |

`session` is a UUID you choose for each call. Ignore messages for any other session. Every payload is at most 64 KiB.

## A Python receiver

[aiortc](https://github.com/aiortc/aiortc) is a WebRTC stack for Python. This receiver calls rover 0, saves one frame as a PNG, prints the frame rate, and hangs up.

```sh
python3 -m pip install 'eclipse-zenoh>=1.10.1,<1.11' aiortc
```

```python
import asyncio, json, time, uuid
import zenoh
from aiortc import RTCPeerConnection, RTCSessionDescription
from aiortc.mediastreams import MediaStreamError
from aiortc.sdp import candidate_from_sdp

BASE = "terra/rover/0/webrtc"


async def watch(seconds=10):
    loop = asyncio.get_running_loop()
    inbox = asyncio.Queue()
    config = zenoh.Config()
    config.insert_json5("mode", '"client"')
    config.insert_json5("connect/endpoints", '["tcp/127.0.0.1:7447"]')
    session = zenoh.open(config)
    call = str(uuid.uuid4())

    def receive(sample):
        loop.call_soon_threadsafe(
            inbox.put_nowait, (str(sample.key_expr), json.loads(sample.payload.to_string())))

    for topic in ("offer", "ice/phone", "state"):
        session.declare_subscriber(f"{BASE}/{topic}", receive)

    def send(topic, **body):
        session.put(f"{BASE}/{topic}", json.dumps({"v": 1, "session": call, **body}))

    peer = RTCPeerConnection()
    frames = []

    @peer.on("track")
    def on_track(track):
        async def pull():
            while True:
                try:
                    frame = await track.recv()
                except MediaStreamError:  # the call ended
                    return
                frames.append(time.monotonic())
                if len(frames) == 30:
                    frame.to_image().save("fpv.png")
        asyncio.ensure_future(pull())

    send("request", action="start")
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            key, message = await asyncio.wait_for(inbox.get(), 0.5)
        except asyncio.TimeoutError:
            continue
        if message.get("session", "").lower() != call:
            continue
        if key.endswith("/offer"):
            await peer.setRemoteDescription(RTCSessionDescription(message["sdp"], "offer"))
            await peer.setLocalDescription(await peer.createAnswer())
            send("answer", sdp=peer.localDescription.sdp)
            send("ice/dashboard", candidate="", sdpMid="0", sdpMLineIndex=0)
        elif key.endswith("/ice/phone") and message["candidate"]:
            candidate = candidate_from_sdp(message["candidate"].removeprefix("candidate:"))
            candidate.sdpMid = message.get("sdpMid")
            candidate.sdpMLineIndex = message["sdpMLineIndex"]
            await peer.addIceCandidate(candidate)
        elif key.endswith("/state"):
            print("call", message["state"])

    send("request", action="stop")
    await peer.close()
    session.close()
    if len(frames) > 1:
        print(f"{len(frames)} frames, {(len(frames) - 1) / (frames[-1] - frames[0]):.1f} fps")


asyncio.run(watch())
```

aiortc puts every candidate in its answer SDP, so the receiver sends only the end-of-candidates message. A browser or `webrtc-rs` dashboard can trickle candidates on `ice/dashboard` as they arrive.

## Networks

ICE uses host candidates only, which is enough on one computer or one LAN. Set `ZORVANE_FPV_STUN` to comma-separated `stun:` or `stuns:` URLs, such as `stun:stun.l.google.com:19302`, to add server-reflexive candidates. The camera renders only while a call is live, and `ZORVANE_FPV=0` turns video off.
