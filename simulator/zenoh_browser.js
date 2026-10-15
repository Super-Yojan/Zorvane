// Pose publisher for the wasm build. Loaded from the play page as a module.
// The Rust Zenoh crate cannot compile to wasm. This file uses zenoh-ts, which
// talks to zenohd through the remote-api plugin over WebSocket.
// zenoh_ts_bundle.js is @eclipse-zenoh/zenoh-ts 1.10.1 with its key-expression
// wasm inlined, because the published package's wasm import does not run in a
// browser without a bundler. The simulator still runs if this file fails to
// load; pose publishes stay quiet until `window.zorvaneZenoh` exists.

import { Config, Encoding, Session } from "./zenoh_ts_bundle.js";

let session = null;
let opening = null;
let openLocator = "";
const publishers = new Map();

function locator(url) {
  const trimmed = url.trim();
  if (trimmed.startsWith("ws://")) {
    return `ws/${authority(trimmed.slice("ws://".length), 80)}`;
  }
  if (trimmed.startsWith("wss://")) {
    return `wss/${authority(trimmed.slice("wss://".length), 443)}`;
  }
  if (trimmed.startsWith("ws/") || trimmed.startsWith("wss/")) {
    return trimmed;
  }
  throw new Error("browser router must start with ws:// or wss://");
}

function authority(rest, fallbackPort) {
  const slash = rest.indexOf("/");
  const host = slash === -1 ? rest : rest.slice(0, slash);
  if (host.includes(":")) {
    return host;
  }
  return `${host}:${fallbackPort}`;
}

async function openSession(url) {
  const next = locator(url);
  if (session && openLocator === next) {
    return;
  }
  if (session) {
    const previous = session;
    session = null;
    publishers.clear();
    openLocator = "";
    try {
      await previous.close();
    } catch (_error) {
      // Replacing a dead session should still open the new locator.
    }
  }
  const opened = await Session.open(new Config(next));
  session = opened;
  openLocator = next;
}

const api = {
  connect(url) {
    const target = locator(url);
    if (session && openLocator === target) {
      return;
    }
    if (opening) {
      return;
    }
    opening = openSession(url)
      .catch((error) => {
        console.warn("Zorvane Zenoh connect failed", error);
        session = null;
        openLocator = "";
      })
      .finally(() => {
        opening = null;
      });
  },
  ready() {
    return session !== null;
  },
  publishPose(id, x, y, heading) {
    if (!session) {
      return;
    }
    const key = `terra/rover/${id}/pose`;
    let publisher = publishers.get(id);
    if (!publisher) {
      publisher = session.declarePublisher(key);
      publishers.set(id, publisher);
    }
    const body = JSON.stringify({ x, y, yaw: heading, heading });
    Promise.resolve(publisher)
      .then((declared) => {
        publishers.set(id, declared);
        return declared.put(body, { encoding: Encoding.APPLICATION_JSON });
      })
      .catch((error) => {
        publishers.delete(id);
        console.warn("Zorvane Zenoh pose publish failed", error);
      });
  },
};

window.zorvaneZenoh = api;
