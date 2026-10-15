# zorvane-client

Subscribe to `terra/rover/<id>/pose` from Python. The matching Rust crate is `crates/zorvane-client`.

```sh
python3 -m pip install -e clients/python
python3 -c "import zorvane; client = zorvane.connect(); client.subscribe_pose(0, print)"
```

After this repo is on the default branch, the same package installs with:

```sh
python3 -m pip install "git+https://github.com/Super-Yojan/Zorvane.git#subdirectory=clients/python"
```

`ws://` is rejected. Pass `tcp/127.0.0.1:7447`, or leave the router unset and the helper probes that port. The browser simulator is the one that uses `ws://localhost:10000`.
