# Reviewer Value v1 tooling

`reviewer_value_v1.py` derives the checked-in product-value proxy from the frozen
ResumeBench-GitHub-Live v1 evaluation. It never reads source code, contacts GitHub, or runs the
StrataDiff producer.

```console
python3 tools/reviewer-value-v1/reviewer_value_v1.py verify
python3 tools/reviewer-value-v1/reviewer_value_v1.py evaluate \
  --output /tmp/reviewer-value-v1.json
```

`verify` recomputes every field and requires byte-for-byte canonical JSON. The claim boundary is
part of the artifact: these are file-count proxies over five selected histories, not measurements
of reviewer time, defect recall, or prevalence.
