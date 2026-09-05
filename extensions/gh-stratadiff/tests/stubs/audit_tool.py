import os
import sys
from pathlib import Path


arguments = sys.argv[1:]
with open(os.environ["STRATADIFF_EXTENSION_TEST_LOG"], "a", encoding="utf-8") as log:
    log.write("audit-tool")
    for argument in arguments:
        log.write(f" {argument}")
    log.write("\n")

command = arguments[0]
if "--output" in arguments:
    output_index = arguments.index("--output") + 1
    if command == "audit":
        output = '{"schema":"stratadiff-review-memory-audit-v2","summary":{"status":"affected"}}\n'
    elif command == "inbox":
        output = '{"schema":"stratadiff-review-inbox-v1","summary":{"status":"actionable"}}\n'
    else:
        raise AssertionError(f"unexpected command: {command}")
    Path(arguments[output_index]).write_text(output, encoding="utf-8")
else:
    if command == "audit":
        print("# Review Memory Audit")
    elif command == "inbox":
        print("# StrataDiff Review Inbox")
    else:
        raise AssertionError(f"unexpected command: {command}")

if command == "inbox" and "INBOX_STUB_EXIT_STATUS" in os.environ:
    print("inbox backend stdout")
    print("inbox backend stderr", file=sys.stderr)
    exit_status = int(os.environ["INBOX_STUB_EXIT_STATUS"])
else:
    exit_status = int(os.environ.get("AUDIT_STUB_EXIT_STATUS", "0"))
raise SystemExit(exit_status)
