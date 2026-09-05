use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use stratadiff::{Language, analyze_bytes};

fn python_module(functions: usize, changed_stride: Option<usize>) -> Vec<u8> {
    let mut source = String::new();
    for index in 0..functions {
        let constant = match changed_stride {
            Some(stride) if index % stride == 0 => index + 1,
            _ => index,
        };
        source.push_str(&format!(
            "def function_{index}(value):\n    adjusted = value + {constant}\n    return adjusted * 2\n\n"
        ));
    }
    source.into_bytes()
}

fn duplicate_python_module(functions: usize) -> Vec<u8> {
    "def repeated():\n    return 1\n\n"
        .repeat(functions)
        .into_bytes()
}

fn structural_diff(c: &mut Criterion) {
    let mut group = c.benchmark_group("python_structural_diff");
    for functions in [100, 1_000, 3_000] {
        let before = python_module(functions, None);
        let after = python_module(functions, Some(50));
        group.throughput(Throughput::Bytes((before.len() + after.len()) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(functions),
            &(before, after),
            |bencher, (before, after)| {
                bencher.iter_batched(
                    || (before.clone(), after.clone()),
                    |(before, after)| {
                        analyze_bytes(
                            black_box(before),
                            black_box(after),
                            "before.py".to_owned(),
                            "after.py".to_owned(),
                            Language::Python,
                        )
                        .unwrap()
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();

    let mut duplicates = c.benchmark_group("python_duplicate_siblings");
    for functions in [100, 1_000, 5_000] {
        let source = duplicate_python_module(functions);
        duplicates.throughput(Throughput::Bytes((source.len() * 2) as u64));
        duplicates.bench_with_input(
            BenchmarkId::from_parameter(functions),
            &source,
            |bencher, source| {
                bencher.iter_batched(
                    || (source.clone(), source.clone()),
                    |(before, after)| {
                        analyze_bytes(
                            black_box(before),
                            black_box(after),
                            "before.py".to_owned(),
                            "after.py".to_owned(),
                            Language::Python,
                        )
                        .unwrap()
                    },
                    BatchSize::LargeInput,
                );
            },
        );
    }
    duplicates.finish();
}

criterion_group!(benches, structural_diff);
criterion_main!(benches);
