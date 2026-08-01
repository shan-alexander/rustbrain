//! Criterion benches: serde_json vs jshift on rustbrain-shaped workloads.
//!
//! ```bash
//! cargo bench -p bench --bench serde_vs_jshift
//! ```

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use jshift::{find_value, mutate_value, parse_path, JsonDoc, JsonView, TypedDoc};
use bench::{
    large_catalog_json, sample_bundle, sample_doctor, sample_workspace, BrainBundleLite,
    WorkspaceMeta, WorkspaceMetaView,
};

fn bench_tiny_workspace_meta(c: &mut Criterion) {
    let meta = sample_workspace();
    let bytes = serde_json::to_vec(&meta).unwrap();

    let mut g = c.benchmark_group("01_tiny_workspace_json");
    g.throughput(Throughput::Bytes(bytes.len() as u64));

    g.bench_function("serde_encode", |b| {
        b.iter(|| serde_json::to_vec(black_box(&meta)).unwrap())
    });
    g.bench_function("serde_encode_pretty", |b| {
        b.iter(|| serde_json::to_vec_pretty(black_box(&meta)).unwrap())
    });
    g.bench_function("serde_decode", |b| {
        b.iter(|| serde_json::from_slice::<WorkspaceMeta>(black_box(&bytes)).unwrap())
    });
    g.bench_function("jshift_to_schema_bytes", |b| {
        b.iter(|| {
            let v = WorkspaceMetaView {
                version: meta.version,
                workspace: meta.workspace.clone(),
            };
            v.to_schema_bytes().unwrap()
        })
    });
    g.bench_function("jshift_read_from", |b| {
        b.iter(|| WorkspaceMetaView::read_from(black_box(&bytes)).unwrap())
    });
    g.bench_function("jshift_typeddoc_get_version", |b| {
        b.iter(|| {
            let doc = TypedDoc::from_slice(black_box(&bytes));
            doc.get::<u32>("version").unwrap()
        })
    });
    g.finish();
}

fn bench_doctor_report(c: &mut Criterion) {
    let report = sample_doctor();
    let bytes = serde_json::to_vec(&report).unwrap();
    let pretty = serde_json::to_string_pretty(&report).unwrap();

    let mut g = c.benchmark_group("02_doctor_cli_pretty_json");
    g.throughput(Throughput::Bytes(pretty.len() as u64));

    g.bench_function("serde_to_string_pretty", |b| {
        b.iter(|| serde_json::to_string_pretty(black_box(&report)).unwrap())
    });
    g.bench_function("serde_from_slice", |b| {
        b.iter(|| {
            serde_json::from_slice::<bench::DoctorReportLite>(black_box(&bytes)).unwrap()
        })
    });
    // jshift: sparse field reads from already-encoded doctor JSON (common agent pattern)
    g.bench_function("jshift_typeddoc_get_nodes_and_healthy", |b| {
        b.iter(|| {
            let doc = TypedDoc::from_slice(black_box(&bytes));
            let nodes: u64 = doc.get("nodes").unwrap();
            let healthy: bool = doc.get("healthy").unwrap();
            (nodes, healthy)
        })
    });
    g.finish();
}

fn bench_brainbundle_full(c: &mut Criterion) {
    let mut g = c.benchmark_group("03_brainbundle_full_serde");
    for n in [50usize, 500, 2000] {
        let bundle = sample_bundle(n, n * 2);
        let bytes = serde_json::to_vec(&bundle).unwrap();
        g.throughput(Throughput::Bytes(bytes.len() as u64));

        g.bench_with_input(BenchmarkId::new("serde_encode", n), &bundle, |b, bundle| {
            b.iter(|| serde_json::to_vec(black_box(bundle)).unwrap())
        });
        g.bench_with_input(BenchmarkId::new("serde_decode", n), &bytes, |b, bytes| {
            b.iter(|| serde_json::from_slice::<BrainBundleLite>(black_box(bytes)).unwrap())
        });
        // jshift sparse: only first node id + edge count path (what agents often need)
        g.bench_with_input(
            BenchmarkId::new("jshift_sparse_first_node_id", n),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    let doc = TypedDoc::from_slice(black_box(bytes));
                    let id: String = doc.get("nodes[0].id").unwrap();
                    let ver: u32 = doc.get("version").unwrap();
                    (id, ver)
                })
            },
        );
    }
    g.finish();
}

fn bench_sparse_path_on_large_json(c: &mut Criterion) {
    let mut g = c.benchmark_group("04_large_json_sparse_path");
    for n in [500usize, 5000] {
        let json = large_catalog_json(n);
        g.throughput(Throughput::Bytes(json.len() as u64));

        g.bench_with_input(
            BenchmarkId::new("serde_value_then_index", n),
            &json,
            |b, json| {
                b.iter(|| {
                    let v: serde_json::Value = serde_json::from_slice(black_box(json)).unwrap();
                    let id = v["nodes"][0]["id"].as_str().unwrap().to_string();
                    let ok = v["meta"]["ok"].as_bool().unwrap();
                    (id, ok)
                })
            },
        );
        g.bench_with_input(
            BenchmarkId::new("jshift_typeddoc_two_fields", n),
            &json,
            |b, json| {
                b.iter(|| {
                    let doc = TypedDoc::from_slice(black_box(json));
                    let id: String = doc.get("nodes[0].id").unwrap();
                    let ok: bool = doc.get("meta.ok").unwrap();
                    (id, ok)
                })
            },
        );
        g.bench_with_input(BenchmarkId::new("jshift_find_value", n), &json, |b, json| {
            b.iter(|| {
                let p = parse_path("nodes[0].id");
                find_value(black_box(json), &p).unwrap().len()
            })
        });
    }
    g.finish();
}

fn bench_inplace_mutate(c: &mut Criterion) {
    let mut g = c.benchmark_group("05_inplace_field_patch");
    let base = br#"{"version":1,"workspace":"/tmp/ws","status":"draft","noise":"yyyyyyyyyy"}"#.to_vec();

    g.bench_function("serde_value_set_and_to_vec", |b| {
        b.iter(|| {
            let mut v: serde_json::Value = serde_json::from_slice(black_box(&base)).unwrap();
            v["status"] = serde_json::json!("published");
            serde_json::to_vec(&v).unwrap()
        })
    });
    g.bench_function("jshift_mutate_value_inplace", |b| {
        b.iter(|| {
            let mut buf = black_box(base.clone());
            let path = parse_path("status");
            mutate_value(&mut buf, &path, br#""published""#).unwrap();
            buf
        })
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_tiny_workspace_meta,
    bench_doctor_report,
    bench_brainbundle_full,
    bench_sparse_path_on_large_json,
    bench_inplace_mutate,
);
criterion_main!(benches);
