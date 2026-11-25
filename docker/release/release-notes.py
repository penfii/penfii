#!/usr/bin/env python3
"""Draft GitHub Release notes for a release: its CHANGELOG section, how to install and upgrade, the images and
the checksums.

  docker/release/release-notes.py VERSION --changelog CHANGELOG.md --sha COMMIT [--digest NAME=sha256:...]...
                                  [--checksums SHA256SUMS] [--scan-summary FILE]

VERSION is X.Y.Z. The text is written to standard output; nothing is published.
"""
import argparse, re, sys

ap = argparse.ArgumentParser()
ap.add_argument("version")
ap.add_argument("--changelog", required=True)
ap.add_argument("--sha", required=True)
ap.add_argument("--digest", action="append", default=[])
ap.add_argument("--checksums")
ap.add_argument("--scan-summary")
a = ap.parse_args()
v, tag = a.version, "v" + a.version

text = open(a.changelog, encoding="utf-8").read()
m = re.search(r"^## %s\n(.*?)(?=^## |\Z)" % re.escape(v), text, re.M | re.S)
if not m:
    sys.exit(f"error: CHANGELOG has no '## {v}' section")
section = m.group(1).strip()
section = re.sub(r"^(#{3,}) ", lambda x: "#" * len(x.group(1)) + " ", section, flags=re.M)
# links in the changelog are relative to the repository root: point them at the tag
section = re.sub(r"\]\((?!https?:|#)([^)]+)\)", lambda x: f"](https://github.com/penfii/penfii/blob/{tag}/{x.group(1)})", section)

out = [f"# Penfii {v}", "", section, "", "## Install", "",
       f"Download `penfii-{tag}-docker.zip` below, check it against `SHA256SUMS`, unpack it and follow its `README.md`:", "",
       "```sh", "sha256sum -c SHA256SUMS --ignore-missing", f"unzip penfii-{tag}-docker.zip && cd penfii",
       "cp .env.example .env   # replace every CHANGE_ME", "docker compose pull && docker compose up -d", "```", "",
       "No source checkout, Rust or Node is needed. Images are `linux/amd64` only.", "",
       "## Upgrade", "",
       "Back up PostgreSQL and the file store first, then follow *Upgrade* in the ZIP's `README.md` and the "
       f"[self-hosting guide](https://github.com/penfii/penfii/blob/{tag}/docs/self-hosting.md#updating). "
       "There is no down migration; going back means restoring the backup.", "",
       "## Images", "", f"Built from commit `{a.sha}` (tag `{tag}`).", "",
       "| Image | Tag | Digest |", "| --- | --- | --- |"]
digests = dict(d.split("=", 1) for d in a.digest)
for name in ("core-api", "web", "opensearch"):
    out.append(f"| `ghcr.io/penfii/{name}` | `{v}` | `{digests.get(name, 'not recorded')}` |")
out += ["", "Each image carries `org.opencontainers.image.revision`, `.version` and `.source` labels, and a "
        "provenance and SBOM attestation. The CycloneDX SBOMs and the Grype report of this release are attached as `sbom-*` assets.", ""]
if a.scan_summary:
    out += ["### Vulnerability report (Grype, report only)", "", "```", open(a.scan_summary, encoding="utf-8").read().strip(), "```", ""]
if a.checksums:
    out += ["## Checksums (SHA-256)", "", "```", open(a.checksums, encoding="utf-8").read().strip(), "```", ""]
print("\n".join(out))
