#!/usr/bin/env python3
"""Fails when docker/install/compose.yaml differs from docker/compose.yaml in anything but the images.

Both files are rendered by `docker compose config --format json` with docker/.env.example, then the parts that are
meant to differ are removed (a service's `build`, its `image`, and the absolute paths of bind mounts), and what is
left must be equal: services, dependencies, networks, volumes, capabilities, read-only file systems, health checks
and settings. Run from the repository root:  docker/release/check-install-compose.py
"""
import json, os, subprocess, sys

root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
env = os.path.join(root, "docker/.env.example")

def render(path):
    out = subprocess.run(["docker", "compose", "--env-file", env, "-f", path, "config", "--format", "json"],
                         check=True, capture_output=True, text=True, cwd=root).stdout
    return json.loads(out)

def normal(cfg):
    for svc in cfg["services"].values():
        svc.pop("build", None)
        svc.pop("image", None)
        for v in svc.get("volumes", []):
            if v.get("type") == "bind":
                v["source"] = os.path.basename(v["source"])
    return cfg

a = normal(render(os.path.join(root, "docker/compose.yaml")))
b = normal(render(os.path.join(root, "docker/install/compose.yaml")))
if a != b:
    for key in sorted(set(a) | set(b)):
        if a.get(key) != b.get(key):
            print(f"differs: {key}", file=sys.stderr)
            if key == "services":
                for s in sorted(set(a[key]) | set(b[key])):
                    if a[key].get(s) != b[key].get(s):
                        print(f"  service {s}:\n    source : {json.dumps(a[key].get(s), sort_keys=True)}\n    install: {json.dumps(b[key].get(s), sort_keys=True)}", file=sys.stderr)
    sys.exit(1)
images = {n: s["image"] for n, s in render(os.path.join(root, "docker/install/compose.yaml"))["services"].items()}
print("ok: install compose equals docker/compose.yaml except images:", json.dumps(images, sort_keys=True))
