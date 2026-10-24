#!/bin/bash
# Fetch the single reviewed source for every Terra crate, including observability.
set -euo pipefail
# Windows runners name the interpreter python, not python3.
python=$(command -v python3 >/dev/null && python3 -c '' 2>/dev/null && echo python3 || echo python)
revision=$("$python" -c 'import json; print(json.load(open("terra-source.json"))["revision"])')
if [[ -e ../Terra ]]; then
    [[ $(git -C ../Terra rev-parse HEAD) == "$revision" ]] || { echo 'Existing sibling Terra differs from the reviewed Terra runtime source' >&2; exit 1; }
else
    git init ../Terra
    git -C ../Terra remote add origin https://github.com/Super-Yojan/Terra.git
    git -C ../Terra fetch --depth=1 origin "$revision"
    git -C ../Terra checkout --detach FETCH_HEAD
fi
