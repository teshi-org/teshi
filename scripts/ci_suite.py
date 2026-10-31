"""Compiled-test inventory, shard execution, artifact identity and fail-closed gates."""

from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import time

ROOT = Path(__file__).resolve().parents[1]
SPEC = ROOT / ".github/terminal-partitions.json"
FEATURE = "features/zh-CN/项目目录加载.feature"
MANIFEST = "crates/teshi-tui-runner/Cargo.toml"

spec = importlib.util.spec_from_file_location("ci_command", ROOT / "scripts/run-ci-command.py")
command_runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(command_runner)


def digest(path):
    with Path(path).open("rb") as source:
        return digest_stream(source)


def digest_stream(source):
    result = hashlib.sha256()
    for chunk in iter(lambda: source.read(1024 * 1024), b""):
        result.update(chunk)
    return result.hexdigest()


def read(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def write(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding="utf-8")


def inventory_digest(inventory):
    return hashlib.sha256(json.dumps(inventory, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")).hexdigest()


def ownership(root=ROOT, specification=None):
    specification = specification or read(root / ".github/terminal-partitions.json")
    shards = specification["shards"]
    if set(shards) != {"terminal-a", "terminal-b"}:
        raise ValueError("both terminal shards are required")
    assigned = [target for targets in shards.values() for target in targets]
    modules = {path.stem for path in (root / "crates/teshi-tui-runner/tests").glob("*.rs")}
    if len(assigned) != len(set(assigned)) or set(assigned) != modules:
        raise ValueError(f"partition ownership differs from test modules: missing={modules-set(assigned)}, extra={set(assigned)-modules}, duplicates={len(assigned)-len(set(assigned))}")
    return {target: shard for shard, targets in shards.items() for target in targets}


def identity():
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True, timeout=10).strip()
    return dict(source_sha=sha, platform=sys.platform, partition_sha256=digest(SPEC))


def manual_tests():
    return read(SPEC).get("manual_tests", {})


def is_manual(target, name):
    return f"{target['id']}::{name}" in manual_tests()


def deferred_tests(inventory, shard):
    return [dict(binary=target["id"], name=test["name"], status="not-run", reason=manual_tests()[f"{target['id']}::{test['name']}"])
            for target in inventory["targets"] if target["owner"] == shard
            for test in target["tests"] if is_manual(target, test["name"])]


def listed_tests(executable, ignored=False):
    command = [str(executable), "--list"] + (["--ignored"] if ignored else [])
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", timeout=30, check=True)
    names = [line[:-6] for line in result.stdout.splitlines() if line.endswith(": test")]
    if len(names) != len(set(names)) or not re.search(r"\d+ tests?, \d+ benchmarks?", result.stdout):
        raise ValueError(f"invalid compiled libtest inventory: {executable}")
    return names


def cargo_artifacts(output):
    artifacts = []
    for line in Path(output).read_text(encoding="utf-8-sig").splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("reason") == "compiler-artifact" and record.get("executable"):
            artifacts.append(record)
    return artifacts


def prepare(artifacts):
    owners = ownership()
    artifacts = Path(artifacts)
    artifacts.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    commands = [
        ("cli-build", ["cargo", "build", "-p", "teshi-cli", "--locked"]),
        ("runner-build", ["cargo", "test", "--manifest-path", MANIFEST, "--locked", "--no-run", "--message-format=json"]),
    ]
    if os.name == "nt":
        commands.append(("vt-build", ["cargo", "test", "-p", "teshi-tui", "--lib", "--locked", "--no-run", "--message-format=json"]))
    for label, command in commands:
        if command_runner.run(command, artifacts / label, label, 1200):
            raise RuntimeError(f"{label} failed; see {artifacts/label}")
    inventory = dict(schema_version=1, **identity(), build_root=str(ROOT), targets=[], binaries={},
                     directory_feature=FEATURE, preparation_seconds=time.monotonic()-started)
    records = cargo_artifacts(artifacts / "runner-build/output.log")
    test_targets = [record for record in records if record["profile"]["test"]]
    compiled_modules = {record["target"]["name"] for record in test_targets if "test" in record["target"]["kind"]}
    if compiled_modules != set(owners):
        raise ValueError(f"compiled integration targets differ from partitions: {compiled_modules ^ set(owners)}")
    for record in test_targets:
        binary = Path(record["executable"])
        target = record["target"]
        kind = target["kind"][0]
        names = listed_tests(binary)
        ignored = set(listed_tests(binary, ignored=True))
        if ignored - set(names):
            raise ValueError("ignored tests are missing from full inventory")
        if ignored and kind != "test":
            raise ValueError(f"unassigned ignored tests in {kind}:{target['name']}")
        inventory["targets"].append(dict(id=f"{kind}:{target['name']}", name=target["name"], kind=kind,
            executable=binary.relative_to(ROOT).as_posix(), sha256=digest(binary),
            owner=owners.get(target["name"]) if kind == "test" else None,
            tests=[dict(name=name, ignored=name in ignored) for name in names]))
    suffix = ".exe" if os.name == "nt" else ""
    inventory["binaries"]["teshi"] = f"target/debug/teshi{suffix}"
    compiled_names = {f"{target['id']}::{test['name']}" for target in inventory["targets"] for test in target["tests"]}
    if set(manual_tests()) - compiled_names:
        raise ValueError("manual-acceptance policy contains tests absent from compiled inventory")
    # Tests refer to Cargo's compile-time CARGO_BIN_EXE path. Preserve the layout.
    for name in ("teshi-tui-runner", "teshi-directory-feature-runner"):
        inventory["binaries"][name] = f"crates/teshi-tui-runner/target/debug/{name}{suffix}"
    if os.name == "nt":
        vt = [record for record in cargo_artifacts(artifacts / "vt-build/output.log")
              if record["profile"]["test"] and record["target"]["name"] == "teshi_tui"]
        if len(vt) != 1:
            raise ValueError("expected exactly one Windows VT fixture artifact")
        inventory["binaries"]["vt"] = Path(vt[0]["executable"]).relative_to(ROOT).as_posix()
    plan = subprocess.check_output([str(ROOT / inventory["binaries"]["teshi-directory-feature-runner"]),
                                    "--inventory", FEATURE], cwd=ROOT, timeout=30)
    inventory["directory_plan"] = json.loads(plan)
    code = command_runner.run(["cargo", "test", "--manifest-path", MANIFEST, "--locked", "--doc", "--", "--list"],
                              artifacts / "doc-inventory", "doc-inventory", 300)
    if code:
        raise RuntimeError("Rustdoc inventory failed")
    inventory["doc_tests"] = [line[:-6] for line in (artifacts / "doc-inventory/output.log").read_text(encoding="utf-8").splitlines() if line.endswith(": test")]
    paths = {target["executable"] for target in inventory["targets"] if any(test["ignored"] for test in target["tests"])}
    paths.update(inventory["binaries"].values())
    inventory["files"] = {path: digest(ROOT / path) for path in sorted(paths)}
    source_paths = {ROOT / FEATURE, ROOT / MANIFEST, ROOT / "crates/teshi-tui-runner/Cargo.lock"}
    source_paths.update((ROOT / "crates/teshi-tui-runner/src").rglob("*.rs"))
    source_paths.update((ROOT / "crates/teshi-tui-runner/tests").rglob("*.rs"))
    source_paths.update(ROOT / path for path in ("crates/teshi-tui/src/lib.rs", "crates/teshi-tui/src/test_frame.rs"))
    source_paths.update(path for path in (ROOT / "crates/teshi-tui-runner/tests/fixtures").rglob("*")
                        if path.is_file() and "__pycache__" not in path.parts)
    source_paths.update(ROOT / f"scripts/{name}" for name in ("ci_suite.py", "ci_process.py", "run-ci-command.py", "run-ci-suite.py"))
    inventory["source_files"] = {path.relative_to(ROOT).as_posix(): digest(path) for path in sorted(source_paths)}
    inventory["preparation_seconds"] = time.monotonic() - started
    write(artifacts / "inventory.json", inventory)
    return inventory


def assert_identity(inventory):
    for key, value in identity().items():
        if inventory[key] != value:
            raise ValueError(f"inventory {key} mismatch")
    for path, checksum in inventory.get("source_files", {}).items():
        if digest(ROOT / path) != checksum:
            raise ValueError(f"inventory source changed: {path}")


def assert_observed(target, observations, mode):
    expected = {test["name"]: test["ignored"] for test in target["tests"]
                if mode == "backend" or (test["ignored"] and not is_manual(target, test["name"]))}
    counts = Counter(item["name"] for item in observations)
    if counts != Counter({name: 1 for name in expected}):
        raise ValueError(f"{target['id']} execution differs from compiled inventory: {counts - Counter(expected)}, missing={set(expected)-set(counts)}")
    for item in observations:
        allowed = item["result"].startswith("ignored") if mode == "backend" and expected[item["name"]] else item["result"] == "ok"
        if not allowed:
            raise ValueError(f"{target['id']} {item['name']} did not execute successfully: {item['result']}")


def configure_environment(inventory, artifacts):
    artifacts.mkdir(parents=True, exist_ok=True)
    os.environ["TESHI_CAPTURE_TIMINGS"] = str(artifacts / "capture-timings.ndjson")
    os.environ["TESHI_ENFORCE_TEST_ISOLATION"] = "1"
    os.environ["TESHI_SELF_TEST_BIN"] = str(ROOT / inventory["binaries"]["teshi"])
    if "vt" in inventory["binaries"]:
        os.environ["TESHI_VT_INPUT_TEST_BIN"] = str(ROOT / inventory["binaries"]["vt"])
    for key, directory in {
        "AI_POINTER": "ai-pointer", "EXPLORE_POINTER": "explore-pointer", "MANAGEMENT_POINTER": "management-pointer",
        "NAVIGATION": "navigation", "OVERLAY": "overlay", "REQUIREMENTS_POINTER": "requirements-pointer",
        "ACP_DISCOVERY": "acp-discovery", "VT_INPUT": "vt-input",
        "ACTIVITY": "activity", "CHAT_FOCUS": "chat-focus", "CHAT_TITLE": "chat-title",
        "CHAT_RUNTIME": "chat-runtime", "CHAT": "chat-selection", "P4_P5": "interaction",
        "REQUIREMENT_CREATION": "requirement-creation", "REQUIREMENT_DELETION": "requirement-deletion",
        "REQUIREMENT_PROJECT": "requirement-project", "TESTPOINTS": "testpoints",
    }.items():
        os.environ[f"TESHI_{key}_ARTIFACTS"] = str(artifacts / directory)


def execute(inventory, shard, artifacts):
    assert_identity(inventory)
    artifacts = Path(artifacts).resolve()
    configure_environment(inventory, artifacts)
    execution = dict(schema_version=1, **identity(), shard=shard, targets=[], status="failed",
                     inventory_sha256=inventory_digest(inventory), deferred=deferred_tests(inventory, shard), errors=[])
    write(artifacts / "execution.json", execution)
    if shard == "directory":
        os.environ["TESHI_FEATURE_ARTIFACTS"] = str(artifacts / "directory")
        cli = os.environ["TESHI_SELF_TEST_BIN"]
        cli_args = [cli, "--requirements-root", str(artifacts / "requirements")]
        check = command_runner.run(cli_args + ["check", "--feature", FEATURE, "--json"], artifacts / "check", "feature-check", 60)
        if check:
            raise RuntimeError("directory Feature validation failed")
        code = command_runner.run(cli_args + ["run", FEATURE, "--runner-cmd", str(ROOT/inventory["binaries"]["teshi-directory-feature-runner"]),
                                   "--report", str(artifacts/"directory/report.json")], artifacts / "command", "directory", 1200)
        if code:
            raise RuntimeError("directory replay failed")
        verify_directory(inventory["directory_plan"], artifacts / "directory")
    else:
        for target in inventory["targets"]:
            expected = [test for test in target["tests"] if shard == "backend" or (test["ignored"] and not is_manual(target, test["name"]))]
            if not expected or (shard != "backend" and target["owner"] != shard):
                continue
            binary = ROOT / target["executable"]
            if digest(binary) != target["sha256"]:
                raise ValueError(f"test executable changed: {target['id']}")
            location = artifacts / "tests" / target["id"].replace(":", "-")
            args = [str(binary), "--test-threads=1"] + ([] if shard == "backend" else ["--ignored"])
            if shard != "backend":
                for test in target["tests"]:
                    if is_manual(target, test["name"]):
                        args.extend(["--skip", test["name"]])
            code = command_runner.run(args, location, target["id"], 600, serial_libtest=True)
            observed = read(location / "timings.json")["tests"]
            execution["targets"].append(dict(id=target["id"], tests=observed, command_status=code))
            try:
                assert_observed(target, observed, shard)
                if code:
                    raise ValueError(f"{target['id']} command failed with {code}")
            except ValueError as error:
                # Each executable has finished owned-tree cleanup. Continue
                # independent binaries to retain all failures, without retries.
                execution["errors"].append(str(error))
            write(artifacts / "execution.json", execution)
        if shard == "backend":
            location = artifacts / "doctests"
            code = command_runner.run(["cargo", "test", "--manifest-path", MANIFEST, "--locked", "--doc", "--", "--test-threads=1"],
                                      location, "doctests", 300, serial_libtest=True)
            execution["doctests"] = read(location / "timings.json")["tests"]
            target = dict(id="doc:teshi-tui-runner", tests=[dict(name=name, ignored=False) for name in inventory["doc_tests"]])
            try:
                assert_observed(target, execution["doctests"], "backend")
                if code:
                    raise ValueError("Rustdoc tests failed")
            except ValueError as error:
                execution["errors"].append(str(error))
        if execution["errors"]:
            write(artifacts / "execution.json", execution)
            raise RuntimeError("; ".join(execution["errors"]))
    execution["status"] = "passed"
    write(artifacts / "execution.json", execution)
    return execution


def verify_directory(plan, artifacts):
    request = read(artifacts / "request.json")
    events = [json.loads(line) for line in (artifacts / "binding-events.ndjson").read_text(encoding="utf-8").splitlines()]
    cases = request["cases"]
    if len({case["id"] for case in cases}) != len(cases):
        raise ValueError("directory case IDs are not unique")
    if Counter((case["scenario"], case["line_number"]) for case in cases) != Counter((case["scenario"], case["line_number"]) for case in plan):
        raise ValueError("directory replay omitted or duplicated source scenarios")
    ids = {(case["scenario"], case["line_number"]): case["id"] for case in cases}
    expected, rows = {}, {}
    for case in plan:
        case_id = ids[case["scenario"], case["line_number"]]
        for variant in case["variants"]:
            rows[case_id, variant["attempt"]] = variant["example"]
            for step in variant["steps"]:
                expected[case_id, step["line"], variant["attempt"]] = step["text"]
    starts = [event for event in events if event["type"] == "start_step"]
    ends = [event for event in events if event["type"] == "end_step"]
    captures = [event for event in events if event["type"] == "step_screenshot"]
    for name, entries in (("start", starts), ("end", ends), ("capture", captures)):
        key = lambda event: (event["case_id"], int(event.get("step_line", event.get("step_id", 0))), event.get("attempt", 1))
        observed = Counter(key(event) for event in entries)
        if observed != Counter({item: 1 for item in expected}):
            raise ValueError(f"directory {name} events differ from expanded source steps")
        for event in entries:
            if name == "start" and event["text"] != expected[key(event)]:
                raise ValueError("directory Examples substitution differs from source")
            if name == "start" and event.get("example") != rows[event["case_id"], event.get("attempt", 1)]:
                raise ValueError("directory Examples identity differs from source")
            if name != "start" and event.get("status") != "passed":
                raise ValueError(f"directory {name} did not pass")
    for event in captures:
        if event["capture_status"] != "available":
            raise ValueError("required screenshot is unavailable")
        for field in ("screenshot_file", "svg_file", "text_file"):
            filename = event[field]
            path = artifacts / "captures" / filename
            if Path(filename).name != filename or not path.is_file() or path.stat().st_size == 0:
                raise ValueError(f"missing archived capture: {filename}")
            with path.open("rb") as stream:
                prefix = stream.read(128)
            if field == "screenshot_file" and not prefix.startswith(b"\x89PNG\r\n\x1a\n"):
                raise ValueError(f"invalid PNG capture: {filename}")
            if field == "svg_file" and b"<svg" not in prefix:
                raise ValueError(f"invalid SVG capture: {filename}")
    timings = read(artifacts / "timing-summary.json")
    if Counter((row["case_id"], row["attempt"]) for row in timings) != Counter({row: 1 for row in rows}) or any(row["status"] != "passed" for row in timings):
        raise ValueError("directory Examples rows did not all finish successfully")
    if any(row["example"] != rows[row["case_id"], row["attempt"]] for row in timings):
        raise ValueError("directory Examples result identity differs from source")
    if not events or events[-1]["type"] != "end_run" or events[-1].get("examples_passed") != len(rows) or events[-1].get("failed") != 0 or events[-1].get("examples_failed") != 0:
        raise ValueError("directory run did not complete")
    report = read(artifacts / "report.json")
    if report["status"] != "passed" or report["summary"]["passed"] != len(plan) or report["summary"]["total"] != len(plan) or any(report["summary"].get(key, 0) for key in ("failed", "skipped", "blocked")):
        raise ValueError("directory CLI report did not pass every scenario")


def pack(inventory, destination):
    started = time.monotonic()
    with tarfile.open(destination, "w:gz", compresslevel=1) as archive:
        for path, checksum in inventory["files"].items():
            if digest(ROOT / path) != checksum:
                raise ValueError(f"binary changed before packaging: {path}")
            archive.add(ROOT / path, arcname=path, recursive=False)
    return dict(elapsed_seconds=time.monotonic()-started, bytes=Path(destination).stat().st_size,
                uncompressed_bytes=sum((ROOT / path).stat().st_size for path in inventory["files"]))


def unpack(inventory, archive_path):
    started = time.monotonic()
    assert_identity(inventory)
    if os.path.normcase(str(ROOT)) != os.path.normcase(inventory["build_root"]):
        raise ValueError("compiled fixture paths require the same checkout root")
    with tarfile.open(archive_path, "r:gz") as archive:
        members = archive.getmembers()
        if Counter(member.name for member in members) != Counter({path: 1 for path in inventory["files"]}) or any(not member.isfile() for member in members):
            raise ValueError("binary archive differs from manifest")
        for member in members:
            destination = (ROOT / member.name).resolve()
            if not destination.is_relative_to(ROOT):
                raise ValueError("archive path escapes checkout")
            with archive.extractfile(member) as source:
                if digest_stream(source) != inventory["files"][member.name]:
                    raise ValueError(f"archive binary identity mismatch: {member.name}")
        archive.extractall(ROOT, filter="data")
    for path, checksum in inventory["files"].items():
        if digest(ROOT / path) != checksum:
            raise ValueError(f"restored binary identity mismatch: {path}")
    return dict(elapsed_seconds=time.monotonic()-started, files=len(inventory["files"]),
                archive_bytes=Path(archive_path).stat().st_size)


def gate(inventory, evidence):
    assert_identity(inventory)
    for shard in ("backend", "directory", "terminal-a", "terminal-b"):
        execution = read(evidence / shard / "execution.json")
        if execution.get("shard") != shard or execution.get("status") != "passed":
            raise ValueError(f"{shard} did not pass")
        for key in ("source_sha", "platform", "partition_sha256"):
            if execution.get(key) != inventory[key]:
                raise ValueError(f"{shard} {key} mismatch")
        if execution.get("deferred", []) != deferred_tests(inventory, shard):
            raise ValueError(f"{shard} manual acceptance accounting differs from policy")
        if execution.get("inventory_sha256") != inventory_digest(inventory):
            raise ValueError(f"{shard} used a different compiled inventory")
        if shard == "directory":
            verify_directory(inventory["directory_plan"], evidence / shard / "directory")
            continue
        expected = {target["id"]: target for target in inventory["targets"]
                    if (shard == "backend" and target["tests"]) or
                    (shard != "backend" and target["owner"] == shard and any(test["ignored"] and not is_manual(target, test["name"]) for test in target["tests"]))}
        if Counter(target["id"] for target in execution["targets"]) != Counter({name: 1 for name in expected}):
            raise ValueError(f"{shard} omitted or duplicated test binaries")
        for target in execution["targets"]:
            if target["command_status"] != 0:
                raise ValueError(f"{target['id']} command failed")
            assert_observed(expected[target["id"]], target["tests"], shard)
        if shard == "backend":
            target = dict(id="doc:teshi-tui-runner", tests=[dict(name=name, ignored=False) for name in inventory["doc_tests"]])
            assert_observed(target, execution["doctests"], "backend")
    return dict(status="passed", **identity(),
        compiled_test_cases=sum(len(target["tests"]) for target in inventory["targets"]),
        required_acceptance_tests=sum(test["ignored"] and not is_manual(target, test["name"]) for target in inventory["targets"] for test in target["tests"]),
        manual_acceptance=deferred_tests(inventory, "terminal-a") + deferred_tests(inventory, "terminal-b"),
        directory_examples=sum(len(case["variants"]) for case in inventory["directory_plan"]),
        directory_steps=sum(len(variant["steps"]) for case in inventory["directory_plan"] for variant in case["variants"]))
