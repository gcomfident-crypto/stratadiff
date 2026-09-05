import os
import sys
from pathlib import Path


arguments = sys.argv[1:]
with open(os.environ["STRATADIFF_EXTENSION_TEST_LOG"], "a", encoding="utf-8") as log:
    log.write("audit-tool")
    for argument in arguments:
        log.write(f" {argument}")
    log.write("\n")

if "--output" in arguments:
    output_index = arguments.index("--output") + 1
    Path(arguments[output_index]).write_text(
        '{"schema":"stratadiff-review-memory-audit-v1","summary":{"status":"affected"}}\n',
        encoding="utf-8",
    )
else:
    print("# Review Memory Audit")

exit_status = int(os.environ.get("AUDIT_STUB_EXIT_STATUS", "0"))
raise SystemExit(exit_status)
