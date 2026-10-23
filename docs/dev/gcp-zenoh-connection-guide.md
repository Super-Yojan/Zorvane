# Connect Zorvane to the GCP Zenoh router

Operator setup guide | 21 October 2026

## 1. Connection settings

Public endpoint: tls/argos.34.135.204.210.sslip.io:7448
Fleet username: terra
Topic prefix: terra/phone
GCP project: glid-app | VM: argos-host | Zone: us-central1-a

Port 7448 is the public, authenticated TLS listener. Port 7449 is restricted to the VM loopback interface. Port 7447 is the simulator's local peer; it is not the hosted router. No VPN is required by this deployment.

## 2. Prepare the source checkout

Install Git, Git LFS, Rust and the native graphics prerequisites described in docs/dev/native-zenoh.md. Run the desktop simulator on a machine with a working GPU. This revision uses sibling Zorvane and Terra directories via .cargo/config.toml.

```bash
git clone https://github.com/Super-Yojan/Terra.git
git -C Terra checkout f1f556c8b4c153a6378dd16db6f00e22cc514a48
git clone https://github.com/Super-Yojan/Zorvane.git
cd Zorvane
git lfs pull
```

If you already have these repositories, preserve your local work and update the existing checkouts instead of cloning again. The Terra commit above is the runtime used to validate this setup.

## 3. Obtain the fleet password

Ask the deployment administrator for the fleet credential through a private channel. An authorized VM administrator can retrieve it from /etc/argos/fleet.password. It is also documented as available in ARGOS's ignored local packaging/gcp/.env file. Do not commit the credential or include it in screenshots.

Fleet credentials are separate from ARGOS operator login permissions. Router access alone does not grant browser/operator roles.

# Start the simulator

## 4. Connect from macOS or Linux

From the Zorvane repository, start a Bash shell and run the following. The password prompt does not echo or place the password in shell history. These settings join the hosted fleet and skip the start menu.

```bash
bash
unset TERRA_ZENOH_CONFIG TERRA_ZENOH_TLS_ROOT_CA
export TERRA_ZENOH=1
export TERRA_ZENOH_CONNECT=\
  tls/argos.34.135.204.210.sslip.io:7448
export TERRA_ZENOH_PREFIX=terra/phone
export TERRA_ZENOH_USER=terra
export TERRA_ZENOH_LISTEN=tcp/127.0.0.1:7447
export TERRA_TILES=1
export ZORVANE_MENU=0
export TERRA_ROVER_COUNT=1
read -r -s -p "Fleet password: " TERRA_ZENOH_PASSWORD
printf "\n"
export TERRA_ZENOH_PASSWORD
cargo run -p zorvane
# After the simulator closes:
unset TERRA_ZENOH_PASSWORD
```

Do not use scripts/run-argos-local.sh for GCP: that launcher deliberately clears hosted credentials and selects the local-only terra/rover prefix. The hosted endpoint uses a public certificate, so this setup does not need a custom root certificate.

## 5. Confirm a real fleet connection

A visible simulator window or a listener on local port 7447 does not prove GCP connectivity. Check the simulator output for fleet connection errors, then confirm that the operator sees fresh pose/status updates for the simulated rover. Expected topic examples are terra/phone/fleet/state and terra/phone/0/pose.

Use distinct numeric rover IDs across simultaneously connected devices. This one-rover simulator starts at ID 0; avoid another device using the same prefix and ID. Start with observation only, then use the operator's normal controls when ready.

# Checks and troubleshooting

## Network and server checks

```bash
nc -vz argos.34.135.204.210.sslip.io 7448

# Authorized VM administrators only:
gcloud compute ssh argos-host \
  --project=glid-app --zone=us-central1-a
sudo systemctl status caddy argos-router argos-server
sudo journalctl -u argos-router --since "10 minutes ago"
```

A successful TCP check proves reachability only, not TLS validation, authentication or telemetry. If unreachable, check the VM state, DNS, GCP ingress rules for TCP 7448 and the local network. Do not expose the VM's internal port 7449.

## Common failures

Authentication failure: verify the terra username and fleet password; re-enter the password. A stale TERRA_ZENOH_CONFIG can override the connection settings, so keep it unset for the command above.

TLS failure: use the hostname exactly as shown, check the client clock and inspect the server certificate renewal service. Do not disable certificate verification. Administrators can inspect argos-router-tls.service and its timer.

No rover data: check terra/phone on both ends, verify the world has started and look for fleet errors. Zorvane can keep its local peer alive even when its separate hosted connection fails. A /readyz response of 503 is expected when the server has no fresh fleet telemetry; /healthz only checks HTTP availability.

Build errors about outdoor or LevelRequest: use the sibling Terra revision listed on page 1. Missing GPU: launch in a normal desktop session with graphics access. Address already in use on 7447: close the other simulator, or set TERRA_ZENOH_LISTEN to an empty string to use only the hosted session.

## Sources and verification scope

Settings were cross-checked against repository deployment configuration, not a live authenticated GCP session. No fleet password is embedded in this document.

Zorvane: docs/dev/native-zenoh.md; scripts/run-argos-local.sh; simulator/src/native_bridge.rs.
ARGOS: packaging/gcp/README.md; packaging/gcp/install.sh; packaging/gcp/argos-router-tls.service.

Source repositories:
https://github.com/Super-Yojan/Zorvane
https://github.com/Super-Yojan/ARGOS
