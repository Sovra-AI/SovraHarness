//! Opt-in synthetic scaling measurements; no models, credentials or personal data.

use std::{hint::black_box, path::PathBuf, time::Instant};

use serde_json::{Value, json};
use sovra_core::{
    ConversationContext,
    teams::{TranscriptLine, VisibleStream},
    terminal::LineDecoder,
};

fn measure(mut operation: impl FnMut()) -> Value {
    operation();
    let samples: Vec<_> = (0..7)
        .map(|_| {
            let started = Instant::now();
            operation();
            started.elapsed().as_nanos()
        })
        .collect();
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    json!({"samples_ns":samples,"median_ns":sorted[3],"max_ns":sorted[6]})
}

#[test]
#[ignore = "synthetic scaling benchmark; set SOVRA_DOMAIN_BENCH_OUTPUT to a fresh JSON path"]
#[expect(
    clippy::disallowed_methods,
    reason = "opt-in measurement harness reads its explicit report destination once"
)]
fn domain_scaling_measurements() {
    let output =
        PathBuf::from(std::env::var("SOVRA_DOMAIN_BENCH_OUTPUT").expect("fresh JSON report path"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    let mut results = Vec::new();
    for bytes in [4_096, 16_384, 65_536, 262_144] {
        let text = "abcdefgh".repeat(bytes / 8);
        for chunk_bytes in [8, 128] {
            let timing = measure(|| {
                let mut stream = VisibleStream::default();
                let mut visible = String::new();
                for chunk in text.as_bytes().chunks(chunk_bytes) {
                    visible.push_str(&stream.push(black_box(std::str::from_utf8(chunk).unwrap())));
                }
                visible.push_str(&stream.finish());
                assert_eq!(visible, text);
                black_box(stream);
            });
            results.push(json!({"workload":"visible_stream","bytes":bytes,
                "chunk_bytes":chunk_bytes,"timing":timing}));
        }
        let terminal_text = "\u{1b}[32mhello world\u{1b}[0m\r\n".repeat(bytes / 22);
        let timing = measure(|| {
            let mut decoder = LineDecoder::default();
            let mut lines = Vec::new();
            for chunk in terminal_text.as_bytes().chunks(128) {
                decoder.push(black_box(chunk), &mut lines);
            }
            decoder.finish(&mut lines);
            assert_ne!(lines.len(), 0);
            assert!(lines.iter().all(|line| line == "hello world"));
            black_box(lines);
        });
        results.push(
            json!({"workload":"terminal_decoder","bytes":terminal_text.len(),
            "chunk_bytes":128,"timing":timing}),
        );
    }
    for messages in [100, 1_000, 10_000, 100_000] {
        let history: Vec<_> = (0..messages)
            .map(|_| TranscriptLine {
                speaker: "User".into(),
                text: "Synthetic context evidence. ".repeat(20),
                source: None,
            })
            .collect();
        let timing = measure(|| {
            let context = ConversationContext::from_team(black_box(&history));
            assert_eq!(context.messages().len(), 24);
            assert!(context.shortened());
            black_box(context);
        });
        results.push(
            json!({"workload":"conversation_context","messages":messages,
            "timing":timing}),
        );
    }
    serde_json::to_writer_pretty(&mut file, &json!({
        "suite":"domain-scaling-v1","status":"complete","warmups":1,
        "samples_per_workload":7,"results":results,
        "limitations":["Synthetic local workloads; excludes rendering, IO and inference.",
            "Timings include output allocations, validation and destruction; fixture creation is excluded.",
            "Run serially and record build profile and host contention separately. No latency pass threshold."]
    })).unwrap();
}
