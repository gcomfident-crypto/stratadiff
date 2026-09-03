# Pinned JDT node enumerator

DiffBenchmark stores exact GumTree/JDT node kinds and UTF-16 ranges. The evaluator therefore uses
the same GumTree JDT generator `3.0.0` with Eclipse JDT Core and ECJ `3.35.0`; it does not infer an
exact JDT kind from a shared parser role.

Build an on-demand local cache with a Java 17 or newer JDK:

```console
JAVA_HOME=/absolute/path/to/jdk-17 \
  tools/diffbenchmark/jdt/bootstrap.sh /absolute/path/to/jdt-cache
```

The script downloads three pinned artifacts, verifies their SHA-256 digests, removes the JDT 3.26
classes embedded in GumTree's release bundle, compiles the enumerator, and prints the launcher
path. Dependencies are cached outside the repository and are not vendored.

The launcher accepts one or more Java files. Its UTF-8 TSV protocol is intentionally small:

```text
BEGIN<TAB>argument-index
NODE<TAB>exact-JDT-kind<TAB>UTF-16-start<TAB>UTF-16-end
END<TAB>argument-index
```

Any download, digest, JDK, parse, or output failure terminates nonzero. There is no parser fallback.
