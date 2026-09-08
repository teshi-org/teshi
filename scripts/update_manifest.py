"""Generate and validate the application update publication contract."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

TARGETS = (
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
)
MANIFEST = "teshi-bundle.json"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def identity(tag, sha, sequence):
    match = re.fullmatch(r"v(\d+\.\d+\.\d+)(?:-nightly\.(\d{8})\.([0-9a-f]{7}))?", tag)
    if not match or not re.fullmatch(r"[0-9a-f]{40}", sha) or sequence <= 0:
        raise ValueError("Invalid release tag, full SHA, or build sequence")
    version, day, short_sha = match.groups()
    if short_sha and not sha.startswith(short_sha):
        raise ValueError("Nightly tag SHA does not match checkout")
    # Resolve once per workflow, so every target embeds the same build start.
    timestamp = os.environ.get("TESHI_RELEASE_TIMESTAMP")
    if not timestamp:
        raise ValueError("TESHI_RELEASE_TIMESTAMP is required")
    from datetime import datetime
    datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    return dict(semver=version, channel="nightly" if day else "stable",
                git_sha=sha, build_timestamp=timestamp, build_sequence=sequence)


def bundle(root, build, target, kind):
    if target not in TARGETS or kind not in ("portable", "msi", "exe"):
        raise ValueError("Unsupported target/installation kind")
    if kind == "exe" and "windows" not in target:
        raise ValueError("EXE bundles are Windows-only")
    suffix = ".exe" if "windows" in target else ""
    prefix = "bin/" if kind in ("msi", "exe") else ""
    required = [prefix + "teshi" + suffix, prefix + "teshi-update-helper" + suffix]
    if "windows" in target:
        required.append(prefix + "teshi-desktop.exe")
    for name in required:
        if not (root / name).is_file():
            raise ValueError(f"Required shipped executable missing: {name}")
    files = []
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"Bundle must not contain links: {path}")
        if not path.is_file() or path == root / MANIFEST:
            continue
        relative = path.relative_to(root).as_posix()
        if relative.startswith(".teshi-update") or relative == "install" or relative.startswith("install/"):
            raise ValueError("Local update state must not be packaged")
        files.append(dict(path=relative, size=path.stat().st_size,
                          sha256=digest(path), executable=relative in required))
    result = dict(schema=1, identity=build, target=target, kind=kind,
                  layout=1, update_explanation=None, files=files)
    write_json(root / MANIFEST, result)
    return result


INCLUDE_KINDS = ("portable", "msi", "exe")


def parse_include(value):
    """Parse a comma-separated subset of portable, msi, and exe publication kinds."""
    kinds = tuple(part.strip() for part in value.split(",") if part.strip())
    if not kinds or any(kind not in INCLUDE_KINDS for kind in kinds):
        raise ValueError("include must be a comma-separated subset of portable,msi,exe")
    return kinds


def release(dist, build, tag, include=INCLUDE_KINDS):
    assets = []
    if "portable" in include:
        for target in TARGETS:
            extension = "zip" if "windows" in target else "tar.gz"
            name = f"teshi-{tag}-{target}.{extension}"
            path = dist / name
            assets.append(dict(name=name, target=target, kind="portable", size=path.stat().st_size, sha256=digest(path)))
    if "msi" in include:
        name = f"teshi-{tag}-x64.msi"
        path = dist / name
        assets.append(dict(name=name, target=TARGETS[0], kind="msi", size=path.stat().st_size, sha256=digest(path)))
    if "exe" in include:
        name = f"teshi-{tag}-x64-setup.exe"
        path = dist / name
        assets.append(dict(name=name, target=TARGETS[0], kind="exe", size=path.stat().st_size, sha256=digest(path)))
    if not assets:
        raise ValueError("No release assets selected")
    write_json(dist / "update-manifest.json", dict(schema=1, minimum_updater=1, identity=build, tag=tag, assets=assets))
    files = sorted(
        p
        for p in dist.iterdir()
        if p.is_file()
        and (
            p.name.endswith((".zip", ".tar.gz", ".msi"))
            or p.name.endswith("-setup.exe")
            or p.name == "update-manifest.json"
        )
    )
    (dist / "SHA256SUMS").write_text("".join(f"{digest(p)}  {p.name}\n" for p in files), encoding="utf-8")


def verify_release(dist):
    """Reject a publication whose SHA256SUMS or asset hashes disagree with the files on disk."""
    sums = {}
    for line in (dist / "SHA256SUMS").read_text(encoding="utf-8").splitlines():
        value, name = line.split(None, 1)
        sums[name.strip()] = value
    manifest = json.loads((dist / "update-manifest.json").read_text(encoding="utf-8"))
    for asset in manifest["assets"]:
        actual = digest(dist / asset["name"])
        if actual != asset["sha256"] or actual != sums[asset["name"]]:
            raise ValueError(f"Checksum mismatch: {asset['name']}")
    if sums["update-manifest.json"] != digest(dist / "update-manifest.json"):
        raise ValueError("update-manifest.json checksum mismatch")


def main():
    parser = argparse.ArgumentParser(__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    build = modes.add_parser("identity")
    build.add_argument("--tag", required=True)
    build.add_argument("--sequence", required=True, type=int)
    build.add_argument("--output", type=Path, default=Path("build-identity.json"))
    pack = modes.add_parser("bundle")
    pack.add_argument("--root", type=Path, required=True)
    pack.add_argument("--identity", type=Path, required=True)
    pack.add_argument("--target", required=True)
    pack.add_argument("--kind", choices=("portable", "msi", "exe"), required=True)
    publish = modes.add_parser("release")
    publish.add_argument("--dist", type=Path, required=True)
    publish.add_argument("--identity", type=Path, required=True)
    publish.add_argument("--tag", required=True)
    publish.add_argument("--include", default="portable,msi,exe")
    check = modes.add_parser("verify")
    check.add_argument("--dist", type=Path, required=True)
    args = parser.parse_args()
    if args.mode == "identity":
        sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
        value = identity(args.tag, sha, args.sequence)
        package = tomllib.loads(Path("Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]
        if value["semver"] != package["version"]:
            raise ValueError("Release tag and workspace version disagree")
        write_json(args.output, value)
        if "GITHUB_ENV" in os.environ:
            with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as stream:
                day = args.tag.split("-nightly.")[1].split(".")[0] if "-nightly." in args.tag else value["build_timestamp"][:10].replace("-", "")
                values = dict(TESHI_BUILD_CHANNEL=value["channel"], TESHI_GIT_SHA=value["git_sha"],
                              TESHI_BUILD_DATE=day,
                              TESHI_BUILD_TIMESTAMP=value["build_timestamp"], TESHI_BUILD_SEQUENCE=str(value["build_sequence"]))
                stream.writelines(f"{key}={value}\n" for key, value in values.items())
    elif args.mode == "bundle":
        bundle(args.root, json.loads(args.identity.read_text(encoding="utf-8")), args.target, args.kind)
    elif args.mode == "verify":
        verify_release(args.dist)
    else:
        release(
            args.dist,
            json.loads(args.identity.read_text(encoding="utf-8")),
            args.tag,
            include=parse_include(args.include),
        )


if __name__ == "__main__":
    main()
