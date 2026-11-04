#!/usr/bin/env python3
"""Bound a CI command, retain its output, and report observed serial libtest times."""

import argparse
import codecs
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import threading
import time
import xml.etree.ElementTree as ET

from ci_process import OwnedProcess


class TestTimes:
    """Observe libtest's flushed 'test NAME ... RESULT' output without nightly Rust.

    These are pipe-observation wall times, including test teardown. They are
    diagnostics, never a test inventory or proof that a suite was fully executed.
    Only use with --test-threads=1 and captured test output (no --nocapture).
    """

    def __init__(self):
        self.buffer = ""
        self.active = None
        self.tests = []

    def feed(self, text, now):
        self.buffer += text
        while True:
            if self.active is None:
                match = re.search(r"(?:^|\n)test ([^\r\n]+?) \.\.\. ", self.buffer)
                if not match:
                    self.buffer = self.buffer[-4096:]
                    return
                self.active = (match[1], now)
                self.buffer = self.buffer[match.end():]
            match = re.match(r"(ok|FAILED|ignored[^\r\n]*)\r?\n", self.buffer)
            if not match:
                return
            name, started = self.active
            self.tests.append(dict(name=name, result=match[1], elapsed_seconds=max(0, now - started)))
            self.buffer = self.buffer[match.end():]
            self.active = None


def run(command, artifacts, label, timeout, serial_libtest=False):
    artifacts.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    chunks = queue.Queue()
    observed = TestTimes()
    try:
        owned = OwnedProcess(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except OSError as error:
        result = dict(label=label, command=command, timed_out=False, returncode=125,
                      elapsed_seconds=time.monotonic() - started, startup_error=str(error),
                      cleanup_seconds=0, cleanup_error=None, tests=[])
        (artifacts / "timings.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
        (artifacts / "output.log").write_text(str(error), encoding="utf-8")
        suite = ET.Element("testsuite", name=label, tests="1", failures="1")
        ET.SubElement(ET.SubElement(suite, "testcase", name=label), "failure").text = str(error)
        ET.ElementTree(suite).write(artifacts / "junit.xml", encoding="utf-8", xml_declaration=True)
        return 125
    process = owned.process

    def read_output():
        try:
            while data := os.read(process.stdout.fileno(), 4096):
                chunks.put((data, time.monotonic()))
        finally:
            chunks.put((None, time.monotonic()))

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    timed_out = False
    exitcode = None
    cleanup_error = None
    result = dict(label=label, command=command, timeout_seconds=timeout, pid=process.pid,
                  temp={key: os.environ.get(key) for key in ("TEMP", "TMP", "TMPDIR", "RUNNER_TEMP")})
    try:
        with (artifacts / "output.log").open("w", encoding="utf-8") as output:
            while True:
                remaining = started + timeout - time.monotonic()
                if remaining <= 0:
                    timed_out = True
                    break
                try:
                    data, received = chunks.get(timeout=min(remaining, .2))
                except queue.Empty:
                    continue
                if data is None:
                    # EOF need not mean exit: a process can close stdout and hang.
                    try:
                        exitcode = process.wait(timeout=max(.001, started + timeout - time.monotonic()))
                    except subprocess.TimeoutExpired:
                        timed_out = True
                    break
                text = decoder.decode(data)
                output.write(text)
                output.flush()
                sys.stdout.write(text)
                sys.stdout.flush()
                if serial_libtest:
                    observed.feed(text, received)
    finally:
        result.update(timed_out=timed_out, returncode=exitcode, active_test=observed.active,
                      elapsed_seconds=time.monotonic() - started,
                      root_returncode_before_cleanup=process.poll())
        cleanup_started = time.monotonic()
        try:
            owned.stop()
        except Exception as error:
            cleanup_error = str(error)
        reader.join(timeout=1)
        if not reader.is_alive():
            process.stdout.close()
        result.update(cleanup_seconds=time.monotonic() - cleanup_started, cleanup_error=cleanup_error,
                      tests=sorted(observed.tests, key=lambda item: item["elapsed_seconds"], reverse=True),
                      timing_source="serial libtest pipe observation" if serial_libtest else "command wall time")
        (artifacts / "timings.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
        slow = "\n".join(f"{item['elapsed_seconds']:8.3f}s {item['result']:8s} {item['name']}"
                         for item in result["tests"][:20])
        (artifacts / "slow-tests.txt").write_text(slow + "\n", encoding="utf-8")
        suite = ET.Element("testsuite", name=label, tests="1", failures=str(int(timed_out or exitcode != 0 or bool(cleanup_error))))
        test = ET.SubElement(suite, "testcase", name=label, time=f"{result['elapsed_seconds']:.6f}")
        if suite.get("failures") == "1":
            ET.SubElement(test, "failure").text = json.dumps({key: value for key, value in result.items() if key != "tests"})
        ET.ElementTree(suite).write(artifacts / "junit.xml", encoding="utf-8", xml_declaration=True)
        print(f"\n{label}: {result['elapsed_seconds']:.3f}s; cleanup {result['cleanup_seconds']:.3f}s")
        if slow:
            print("Slow tests (observed serial libtest output):\n" + slow)
    return 124 if timed_out else (exitcode or int(bool(cleanup_error)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--label", required=True)
    parser.add_argument("--timeout-seconds", required=True, type=float)
    parser.add_argument("--serial-libtest", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command or not 0 < args.timeout_seconds < float("inf"):
        parser.error("a command and a finite positive timeout are required")
    return run(command, args.artifacts, args.label, args.timeout_seconds, args.serial_libtest)


if __name__ == "__main__":
    sys.exit(main())
