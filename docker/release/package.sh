#!/usr/bin/env bash
# Builds the official install ZIP of a release from a SOURCE CHECKOUT OF ITS TAG plus this tooling.
#
#   docker/release/package.sh VERSION OUTDIR [SRC]
#
# VERSION is X.Y.Z (no v). SRC is the checkout of the release tag (default: the repository this script is in);
# it provides docker/.env.example, docker/minio/setup.sh and LICENSE, so the files of the package are the ones of
# the released source. docker/install/ (compose.yaml, README.md) comes from the tooling's own checkout. Writes
# OUTDIR/penfii-vVERSION-docker.zip and OUTDIR/penfii-vVERSION-docker.zip.sha256. The ZIP is deterministic: the same
# inputs give the same bytes (sorted entries, fixed timestamps and modes).
set -euo pipefail
[ "$#" -ge 2 ] && [ "$#" -le 3 ] || { sed -n '2,10p' "$0"; exit 2; }
VERSION="$1"; OUT="$2"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="${3:-$(cd "$HERE/.." && pwd)}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "error: VERSION must be X.Y.Z" >&2; exit 2; }
for f in "$HERE/install/compose.yaml" "$HERE/install/README.md" "$SRC/docker/.env.example" "$SRC/docker/minio/setup.sh" "$SRC/LICENSE"; do
  [ -f "$f" ] || { echo "error: missing $f" >&2; exit 1; }
done
mkdir -p "$OUT"
python3 -I - "$VERSION" "$HERE/install" "$SRC" "$OUT" <<'PY'
import hashlib, os, re, sys, zipfile
version, install, src, out = sys.argv[1:5]
tag = "v" + version
def read(p): return open(p, encoding="utf-8").read()

compose = read(f"{install}/compose.yaml")
default = "${PENFII_VERSION:-dev}"
if compose.count(default) != 5:
    sys.exit("error: compose.yaml must name the version default exactly 5 times (found %d)" % compose.count(default))
compose = compose.replace(default, "${PENFII_VERSION:-%s}" % version)

env = read(f"{src}/docker/.env.example")
env = env.replace("# Settings for the official Docker Compose install (docker/compose.yaml).",
                  "# Settings for Penfii %s (compose.yaml in this directory)." % version)
env = env.replace("docs/self-hosting.md", f"https://github.com/penfii/penfii/blob/{tag}/docs/self-hosting.md")
env = env.replace("docs/operations.md", f"https://github.com/penfii/penfii/blob/{tag}/docs/operations.md")
env = env.replace("`docker/.env`", "`.env`")
if "CHANGE_ME" not in env:
    sys.exit("error: .env.example has no CHANGE_ME placeholders")
env += ("\n# --- Version -----------------------------------------------------------------------------------\n"
        "# The image tag. compose.yaml already defaults to %s; set this only to pin it explicitly.\n"
        "#PENFII_VERSION=%s\n#PENFII_IMAGE_PREFIX=ghcr.io/penfii\n") % (version, version)

readme = read(f"{install}/README.md").replace("@VERSION@", version)
files = {
    "penfii/compose.yaml": (compose.encode(), 0o644),
    "penfii/.env.example": (env.encode(), 0o644),
    "penfii/minio/setup.sh": (open(f"{src}/docker/minio/setup.sh", "rb").read(), 0o755),
    "penfii/README.md": (readme.encode(), 0o644),
    "penfii/LICENSE": (open(f"{src}/LICENSE", "rb").read(), 0o644),
}
path = f"{out}/penfii-{tag}-docker.zip"
with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for name in sorted(files):
        data, mode = files[name]
        zi = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
        zi.external_attr = (0o100000 | mode) << 16
        zi.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(zi, data, compresslevel=9)
digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
open(path + ".sha256", "w").write(f"{digest}  {os.path.basename(path)}\n")
print(f"{path}\n{digest}  {os.path.basename(path)}")
PY
