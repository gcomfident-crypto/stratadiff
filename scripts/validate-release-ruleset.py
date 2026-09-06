#!/usr/bin/env python3

import json
import sys


if len(sys.argv) != 1:
    raise SystemExit("usage: scripts/validate-release-ruleset.py")

ruleset = json.load(sys.stdin)
if not isinstance(ruleset, dict):
    raise SystemExit(1)

required_fields = {
    "name",
    "target",
    "enforcement",
    "bypass_actors",
    "conditions",
    "rules",
}
if not required_fields.issubset(ruleset):
    raise SystemExit(1)

conditions = ruleset["conditions"]
if not isinstance(conditions, dict) or "ref_name" not in conditions:
    raise SystemExit(1)
ref_name = conditions["ref_name"]
if not isinstance(ref_name, dict) or not {"include", "exclude"}.issubset(ref_name):
    raise SystemExit(1)

rules = ruleset["rules"]
if not isinstance(rules, list) or not all(
    isinstance(rule, dict) and "type" in rule and isinstance(rule["type"], str)
    for rule in rules
):
    raise SystemExit(1)

valid = (
    ruleset["name"] == "Protect immutable v* release tags"
    and ruleset["target"] == "tag"
    and ruleset["enforcement"] == "active"
    and ruleset["bypass_actors"] == []
    and ref_name["include"] == ["refs/tags/v*"]
    and ref_name["exclude"] == []
    and sorted(rule["type"] for rule in rules) == ["deletion", "update"]
)
raise SystemExit(0 if valid else 1)
