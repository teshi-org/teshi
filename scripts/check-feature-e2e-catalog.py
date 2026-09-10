#!/usr/bin/env python3
"""Check that every product Feature has a declared verification owner."""

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
FEATURES = ROOT / "features"
RUNNER_TAGS = {"@api", "@cli", "@validation-e2e", "@web-ui"}
LANGUAGE_BY_DIRECTORY = {"en-US": "# language: en", "zh-CN": "# language: zh-CN"}


def feature_tags(content: str) -> set[str]:
    return {
        token
        for line in content.splitlines()
        if line.strip().startswith("@")
        for token in line.split()
        if token.startswith("@")
    }


def main() -> int:
    errors: list[str] = []
    feature_files = sorted(FEATURES.rglob("*.feature"))
    for path in feature_files:
        relative = path.relative_to(ROOT).as_posix()
        content = path.read_text(encoding="utf-8")
        tags = feature_tags(content)
        owners = tags & RUNNER_TAGS
        if not owners:
            errors.append(f"{relative}: missing runner tag ({', '.join(sorted(RUNNER_TAGS))})")

        locale = path.parent.name
        expected_language = LANGUAGE_BY_DIRECTORY.get(locale)
        if expected_language and expected_language not in content.splitlines()[:3]:
            errors.append(f"{relative}: expected {expected_language!r} in the language header")

        if "@validation-e2e" in tags and "@cli" not in tags:
            errors.append(f"{relative}: @validation-e2e must also declare the CLI surface")

        if "@web-ui" in tags:
            binding_path = path.with_suffix(".bindings.json")
            if not binding_path.is_file():
                errors.append(f"{relative}: @web-ui requires {binding_path.name}")
            else:
                try:
                    binding = json.loads(binding_path.read_text(encoding="utf-8"))
                except (OSError, json.JSONDecodeError) as error:
                    errors.append(f"{relative}: invalid binding sidecar: {error}")
                else:
                    if not isinstance(binding, dict):
                        errors.append(f"{relative}: binding sidecar must contain a JSON object")
                    elif binding.get("feature") != relative:
                        errors.append(
                            f"{relative}: binding sidecar points to {binding.get('feature')!r}"
                        )

        unknown = sorted(tag for tag in tags if tag.startswith("@") and tag not in RUNNER_TAGS)
        if unknown:
            errors.append(f"{relative}: unknown runner tag(s): {', '.join(unknown)}")

    if errors:
        print("Feature E2E catalog check failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(f"Feature E2E catalog check passed: {len(feature_files)} executable Feature files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
