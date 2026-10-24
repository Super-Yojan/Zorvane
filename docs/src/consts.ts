export const SITE_TITLE = "Zorvane";

/** Prefix an in-site path with the project Page base, `/Zorvane`. */
export const href = (path: string) =>
    `${import.meta.env.BASE_URL.replace(/\/$/, "")}${path === "/" ? "/" : path}`;

/** Every page, in reading order. The menu and the index use this list. */
export const pages = [
    { path: "/running", label: "Download and run", blurb: "Get the simulator for macOS, Windows or Linux, and start a match." },
    { path: "/next", label: "The game", blurb: "NEXT 2026 on Field 3: the field, the buckets, scoring and what the simulator models." },
    { path: "/dashboard", label: "Build a dashboard", blurb: "Connect over Zenoh, read the rover, drive it, load and dump." },
    { path: "/topics", label: "Topic reference", blurb: "Every key under terra/rover, with its JSON." },
    { path: "/fpv", label: "FPV video", blurb: "Answer the rover's WebRTC call, the same way ARGOS does on a real rover." },
];
