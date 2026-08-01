//! Criterion benches: rustbrain engine vs intentional alternatives.
//!
//! ```bash
//! cargo bench -p bench --bench rustbrain_performance
//! # quick smoke:
//! cargo bench -p bench --bench rustbrain_performance -- --quick
//! ```
//!
//! ## What we compare (fair framing)
//!
//! | Workload | rustbrain | Alternative (what we did *not* ship as primary) |
//! |----------|-----------|--------------------------------------------------|
//! | Search | FTS5 + ranked boosts | Walk all `.md` + substring; SQLite `LIKE` full scan |
//! | Context pack | Ranked seeds + CSR hops + budget | Path-order concat; grep-rank then concat |
//! | Neighborhood | SQLite edge BFS / CSR k-hop | Re-parse every file’s WikiLinks each query |
//! | WikiLink extract | Fence-aware scanner | Naive `[[…]]` regex (includes code fences) |
//! | Sync | Hash-skip re-index | Force work via touch + sync |
//!
//! Alternatives are **honest baselines**, not peer products. Quality differs
//! (e.g. fence-aware links, BM25, type priors) — times show cost of the
//! “no index / no CSR / no FTS” paths rustbrain exists to avoid.

use bench::{
    densify_plan, grep_ranked_concat_context, naive_concat_context, naive_regex_wikilinks,
    naive_walk_search, naive_wikilink_bfs, rb_query, rebuild_wikilink_adj, sample_markdown_with_links,
    sample_plan_markdown, sqlite_like_search, touch_append, BenchWorkspace, KEYWORD_RAFT,
    KEYWORD_SQLITE,
};
use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use rustbrain_core::{
    extract_wikilinks, CsrMmapGraph, GraphOptions, QueryOptions, Brain, ContextOptions,
};
use std::time::Duration;

/// Default corpus sizes. 200 is the “readme table” size; 500 stress-tests scale.
const SIZES: &[usize] = &[100, 200, 500];

fn configure(g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>) {
    g.sample_size(40);
    g.measurement_time(Duration::from_secs(8));
    g.warm_up_time(Duration::from_secs(1));
}

// ── 01 Search ───────────────────────────────────────────────────────────────

fn bench_search(c: &mut Criterion) {
    let mut g = c.benchmark_group("01_search");
    configure(&mut g);

    for &n in SIZES {
        let ws = BenchWorkspace::create(n).expect("fixture");
        let brain = ws.open_brain().expect("open");
        let db_path = ws.root.join(".brain/db.sqlite");
        let query = format!("{KEYWORD_SQLITE} storage");
        let limit = 25usize;

        g.throughput(Throughput::Elements(n as u64));

        g.bench_with_input(BenchmarkId::new("rustbrain_query_ranked", n), &n, |b, _| {
            b.iter(|| {
                let hits = rb_query(black_box(&brain), black_box(&query), limit).unwrap();
                black_box(hits)
            })
        });

        g.bench_with_input(BenchmarkId::new("alt_walk_md_contains", n), &n, |b, _| {
            b.iter(|| {
                let hits =
                    naive_walk_search(black_box(&ws.root), black_box(&query), limit).unwrap();
                black_box(hits.len())
            })
        });

        g.bench_with_input(BenchmarkId::new("alt_sqlite_like_scan", n), &n, |b, _| {
            b.iter(|| {
                let hits =
                    sqlite_like_search(black_box(&db_path), black_box(&query), limit).unwrap();
                black_box(hits.len())
            })
        });
    }
    g.finish();
}

// ── 02 Context pack ─────────────────────────────────────────────────────────

fn bench_context(c: &mut Criterion) {
    let mut g = c.benchmark_group("02_context_pack");
    configure(&mut g);

    for &n in SIZES {
        let ws = BenchWorkspace::create(n).expect("fixture");
        let brain = ws.open_brain().expect("open");
        let prompt = format!("why {KEYWORD_SQLITE} for local index");
        let max_tokens = 1024usize;
        let max_chars = max_tokens * 4;

        g.throughput(Throughput::Elements(n as u64));

        g.bench_with_input(BenchmarkId::new("rustbrain_context", n), &n, |b, _| {
            b.iter(|| {
                let ctx = brain
                    .context_for_prompt(black_box(&prompt), black_box(max_tokens))
                    .unwrap();
                black_box(ctx.nodes.len())
            })
        });

        g.bench_with_input(
            BenchmarkId::new("alt_path_order_concat", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let (count, chars) =
                        naive_concat_context(black_box(&ws.root), black_box(max_chars)).unwrap();
                    black_box((count, chars))
                })
            },
        );

        g.bench_with_input(
            BenchmarkId::new("alt_grep_rank_then_concat", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let (count, chars) = grep_ranked_concat_context(
                        black_box(&ws.root),
                        black_box(&prompt),
                        black_box(max_chars),
                    )
                    .unwrap();
                    black_box((count, chars))
                })
            },
        );
    }
    g.finish();
}

// ── 03 Graph neighborhood ───────────────────────────────────────────────────

fn bench_graph(c: &mut Criterion) {
    let mut g = c.benchmark_group("03_graph_neighborhood");
    configure(&mut g);

    for &n in SIZES {
        let ws = BenchWorkspace::create(n).expect("fixture");
        let brain = ws.open_brain().expect("open");
        let mmap_path = ws.root.join(".brain/graph.mmap");
        let csr = CsrMmapGraph::open(&mmap_path).expect("mmap");
        let root_id = "docs/concepts/note-0";
        let root_idx = csr.index_of(root_id).expect("root in csr") as usize;
        let opts = GraphOptions {
            hops: 1,
            include_auto: false,
            ..GraphOptions::default()
        };

        g.throughput(Throughput::Elements(n as u64));

        g.bench_with_input(
            BenchmarkId::new("rustbrain_sql_neighborhood", n),
            &n,
            |b, _| {
                b.iter(|| {
                    let nb = brain
                        .graph_neighborhood(black_box(root_id), black_box(&opts))
                        .unwrap();
                    black_box(nb.edges.len())
                })
            },
        );

        g.bench_with_input(BenchmarkId::new("rustbrain_csr_k_hop", n), &n, |b, _| {
            b.iter(|| {
                let hops = csr.k_hop_neighborhood(black_box(root_idx), black_box(1));
                black_box(hops.len())
            })
        });

        g.bench_with_input(
            BenchmarkId::new("alt_reparse_wikilinks_bfs", n),
            &n,
            |b, _| {
                b.iter(|| {
                    // Full cost of “no edge index”: re-read every note, extract links, BFS.
                    let adj = rebuild_wikilink_adj(black_box(&ws.root)).unwrap();
                    let edges = naive_wikilink_bfs(&adj, black_box(root_id), 1);
                    black_box(edges)
                })
            },
        );
    }
    g.finish();
}

// ── 04 WikiLink extraction ──────────────────────────────────────────────────

fn bench_wikilink_extract(c: &mut Criterion) {
    let mut g = c.benchmark_group("04_wikilink_extract");
    g.sample_size(80);
    g.measurement_time(Duration::from_secs(5));

    for n_links in [50usize, 200, 1000] {
        let md = sample_markdown_with_links(n_links);
        g.throughput(Throughput::Bytes(md.len() as u64));

        g.bench_with_input(
            BenchmarkId::new("rustbrain_fence_aware", n_links),
            &n_links,
            |b, _| {
                b.iter(|| {
                    let links = extract_wikilinks(black_box(&md));
                    black_box(links.len())
                })
            },
        );

        g.bench_with_input(
            BenchmarkId::new("alt_naive_regex", n_links),
            &n_links,
            |b, _| {
                b.iter(|| {
                    let count = naive_regex_wikilinks(black_box(&md));
                    black_box(count)
                })
            },
        );
    }
    g.finish();
}

// ── 05 Sync ─────────────────────────────────────────────────────────────────

fn bench_sync(c: &mut Criterion) {
    let mut g = c.benchmark_group("05_sync");
    g.sample_size(25);
    g.measurement_time(Duration::from_secs(10));
    g.warm_up_time(Duration::from_secs(1));

    for &n in &[100usize, 200] {
        // Warm no-op: index already current (content_hash skip path).
        {
            let ws = BenchWorkspace::create(n).expect("fixture");
            let mut brain = ws.open_brain().expect("open");
            g.bench_with_input(BenchmarkId::new("rustbrain_sync_noop", n), &n, |b, _| {
                b.iter(|| {
                    let stats = brain.sync().unwrap();
                    black_box(stats.nodes_upserted)
                })
            });
        }

        // Incremental: append one line each iter, then sync.
        {
            let ws = BenchWorkspace::create(n).expect("fixture");
            let mut brain = ws.open_brain().expect("open");
            let touch = ws.touch_file.clone();
            let mut i = 0u64;
            g.bench_with_input(
                BenchmarkId::new("rustbrain_sync_one_file_dirty", n),
                &n,
                |b, _| {
                    b.iter(|| {
                        i += 1;
                        touch_append(&touch, &format!("bench-{i}")).unwrap();
                        let stats = brain.sync().unwrap();
                        black_box(stats.nodes_upserted)
                    })
                },
            );
        }
    }
    g.finish();
}

// ── 06 Query variants (rustbrain-only micro) ────────────────────────────────

fn bench_query_variants(c: &mut Criterion) {
    let mut g = c.benchmark_group("06_query_variants");
    configure(&mut g);

    let n = 200usize;
    let ws = BenchWorkspace::create(n).expect("fixture");
    let brain = ws.open_brain().expect("open");

    g.bench_function("human_notes_only_sqlite", |b| {
        b.iter(|| {
            rb_query(black_box(&brain), black_box(KEYWORD_SQLITE), 25).unwrap()
        })
    });

    g.bench_function("with_symbols_raft", |b| {
        b.iter(|| {
            let mut opts = QueryOptions::default();
            opts.limit = 25;
            opts.no_symbols = false;
            let hits = brain
                .query_ranked(black_box(KEYWORD_RAFT), black_box(&opts))
                .unwrap();
            black_box(hits.len())
        })
    });

    g.bench_function("natural_language_why_sqlite", |b| {
        b.iter(|| {
            rb_query(
                black_box(&brain),
                black_box("why local sqlite for the index"),
                25,
            )
            .unwrap()
        })
    });

    g.bench_function("plan_status_in_progress", |b| {
        b.iter(|| {
            let mut opts = QueryOptions::human();
            opts.limit = 10;
            opts.include_types = vec![rustbrain_core::NodeType::Plan];
            let hits = brain
                .query_ranked(black_box("status:in_progress"), black_box(&opts))
                .unwrap();
            black_box(hits.len())
        })
    });

    g.bench_function("context_seeds_only_hops0", |b| {
        b.iter(|| {
            let ctx = brain
                .context_for_prompt_with(
                    black_box("sqlite fts"),
                    black_box(&ContextOptions {
                        max_tokens: 1024,
                        hop_depth: 0,
                        ..ContextOptions::default()
                    }),
                )
                .unwrap();
            black_box(ctx.nodes.len())
        })
    });

    g.bench_function("doctor", |b| {
        b.iter(|| {
            let report = rustbrain_core::run_doctor(black_box(&ws.root)).unwrap();
            black_box(report.healthy)
        })
    });

    g.bench_function("densify_plan_micro", |b| {
        let md = sample_plan_markdown();
        b.iter(|| {
            let d = densify_plan(black_box(Some("in_progress")), black_box(md));
            black_box(d.overall)
        })
    });

    g.finish();
}

// ── 07 Cold open ────────────────────────────────────────────────────────────

fn bench_open(c: &mut Criterion) {
    let mut g = c.benchmark_group("07_open_brain");
    g.sample_size(50);
    g.measurement_time(Duration::from_secs(5));

    let n = 200usize;
    let ws = BenchWorkspace::create(n).expect("fixture");

    g.bench_function("open_exact_200_notes", |b| {
        b.iter(|| {
            let brain = Brain::open_exact(black_box(&ws.root)).unwrap();
            black_box(brain.workspace().to_path_buf())
        })
    });

    let mmap_path = ws.root.join(".brain/graph.mmap");
    g.bench_function("open_csr_mmap", |b| {
        b.iter(|| {
            let g = CsrMmapGraph::open(black_box(&mmap_path)).unwrap();
            black_box(g.node_count)
        })
    });

    g.finish();
}

criterion_group!(
    benches,
    bench_search,
    bench_context,
    bench_graph,
    bench_wikilink_extract,
    bench_sync,
    bench_query_variants,
    bench_open,
);
criterion_main!(benches);
