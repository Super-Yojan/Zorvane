# Qwen NEXT gameplay agent

This experimental Python agent connects to Zorvane over Zenoh and asks an
Ollama-hosted Qwen model to choose rover actions. Ollama may run on another
computer; only the Python client package is needed on the simulator computer.
The optional vision model describes the front RGB image and a rendered local
occupancy map for the decision model.

From the repository root:

```sh
python3 -m venv .venv-qwen
.venv-qwen/bin/python -m pip install -r simulator/tools/requirements-qwen.txt
cargo run -p zorvane
```

In another terminal, replace the host and model names with values available
from the Ollama server's `/api/tags` response:

```sh
.venv-qwen/bin/python simulator/tools/qwen_next_agent.py \
  --ollama-host http://100.x.y.z:11434 \
  --model qwen2.5:latest \
  --vision-model qwen2.5vl:latest \
  --forever
```

Omit `--vision-model` to run without camera descriptions. The default Zenoh
endpoint is `tcp/127.0.0.1:7447` and the default rover is 0; use
`--zenoh-endpoint`, `--prefix`, or `--rover` when those differ. Stop with
Ctrl+C; the agent cancels its waypoint and sends a zero drive command.

The decision model receives rover pose, NEXT counts and score, goal and
autonomy status, hardware status, and occupancy metadata. With a vision model,
the Python process sends the latest front RGB frame and a rendered occupancy
grid in a separate Ollama request, then passes its text description to the
decision model. The grid covers observed geometry; it does not identify balls
or buckets. Current Zorvane also publishes detector estimates on
`terra/rover/<id>/objects`; this agent does not yet consume that topic, so
bucket search and precise dumping remain experimental. Routine loading at
home, dumping when aligned, waypoint monitoring, and short recovery moves run
in Python without another model decision.

Run the agent's focused tests with:

```sh
.venv-qwen/bin/python -m unittest discover -s simulator/tools -p 'test_qwen_next_agent.py'
```
