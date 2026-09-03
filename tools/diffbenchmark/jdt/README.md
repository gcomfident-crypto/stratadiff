# Pinned JDT node enumerator

DiffBenchmark stores exact GumTree/JDT node kinds and UTF-16 ranges. The evaluator therefore uses
the same GumTree JDT generator `3.0.0` with Eclipse JDT Core and ECJ `3.35.0`; it does not infer an
exact JDT kind from a shared parser role.

Build an on-demand local cache with a Java 17 or newer JDK:

```console
JAVA_HOME=/absolute/path/to/jdk-17 \
  tools/diffbenchmark/jdt/bootstrap.sh /absolute/path/to/jdt-cache
```

The script downloads three pinned artifacts, verifies their SHA-256 digests, extracts the pinned
GumTree fat JAR, records the canonical Java 17+ executable and a neutral-extension copy of the
helper source under `provenance/`, compiles the helper, and prints the launcher path. The external
JDT and ECJ JARs precede the GumTree fat JAR on the classpath. Dependencies are cached outside the
repository and are not vendored. The local JDK is an explicit trust boundary: it is selected by the
caller, not attested by files inside the cache, and its exact canonical path must be supplied again
when evaluating.

The launcher accepts zero or more Java files. Its UTF-8, LF-only TSV protocol is intentionally
small:

```text
HELLO<TAB>profile<TAB>protocol<TAB>argument-count
BEGIN<TAB>argument-index<TAB>source-SHA-256
NODE<TAB>exact-JDT-kind<TAB>UTF-16-start<TAB>UTF-16-end
END<TAB>argument-index<TAB>node-count
DONE<TAB>block-count<TAB>total-node-count
```

GumTree/JDT can expose multiple preorder nodes with the same exact DiffBenchmark identity: JDT
kind, UTF-16 start, and UTF-16 end. Because that identity cannot distinguish the nodes, the
verified `helper-v3` emits only the first occurrence of each identity. It visits the GumTree root in
preorder, then adds JDT's source-ordered comment table, explicitly mapping Javadoc, line comments,
and block comments to `Javadoc`, `LineComment`, and `BlockComment`. `END` and `DONE` count those
unique emitted identities. The evaluator still rejects duplicate `NODE` records from unverified
enumerators rather than silently deduplicating their output.

Any download, digest, JDK, parse, timeout, or output-limit failure terminates nonzero. There is no
parser fallback. `stratadiff-evaluate --jdt-cache` independently verifies the runtime JARs and
neutral helper source, copies them into a private temporary runtime directory, verifies the copies,
and runs the copied source directly. It does not trust the generated launcher or compiled helper
class. On Unix, the snapshot directory has mode `0700` and each copied JAR or helper file has mode
`0600`. The evaluator caps the JVM heap at 1024 MiB, allows at most 512 MiB stdout, 1 MiB stderr,
and 10,000,000 `NODE` records, and terminates enumeration after 300 seconds. Its Java-version probe
uses a 128 MiB heap cap, 64 KiB limits for each output stream, and a 10-second timeout.

```console
cargo run --release --bin stratadiff-evaluate -- \
  /absolute/path/to/DiffBenchmark /absolute/path/to/materialized \
  --jdt-cache /absolute/path/to/jdt-cache \
  --java-executable /trusted/absolute/path/to/jdk-17/bin/java \
  --require-complete
```

The cache's `provenance/java-executable` line must equal the canonical form of the explicit
`--java-executable` path. The evaluator records the observed Java version and the
`caller_selected_local_executable` trust-boundary marker in its report; it does not claim a digest
for the locally trusted JDK.

An arbitrary launcher is accepted only with both `--jdt-enumerator` and
`--allow-unverified-jdt-enumerator`; such a run reports no dependency versions and can never set
`benchmarkComplete`.

`--require-complete` still writes the JSON report, but exits unsuccessfully unless the verified JDT
cache, canonical materialization manifest, full 285-case selection, and exact pinned outcome all
satisfy `benchmarkComplete`.
