use std::sync::Mutex;

use serde_json::json;
use sovra_core::plugins::CodeFetchReply;

use super::*;

/// Records what the code asked for and answers with a fixed reply.
#[derive(Default)]
struct FakeNetwork {
    asked: Mutex<Vec<String>>,
}

impl CodeHost for FakeNetwork {
    fn fetch(&self, request: CodeFetch) -> Result<CodeFetchReply, String> {
        self.asked
            .lock()
            .expect("test lock is never poisoned")
            .push(request.url);
        Err("blocked for the test".into())
    }
}

fn network() -> Arc<FakeNetwork> {
    Arc::new(FakeNetwork::default())
}

/// A bump allocator and memory every test module shares.
const PRELUDE: &str = r#"
    (memory (export "memory") 1)
    (global $next (mut i32) (i32.const 4096))
    (func (export "sovra_alloc") (param $len i32) (result i32)
      (local $at i32)
      (local.set $at (global.get $next))
      (global.set $next (i32.add (global.get $next) (local.get $len)))
      (local.get $at))
    (func $pack (param $ptr i32) (param $len i32) (result i64)
      (i64.or (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
              (i64.extend_i32_u (local.get $len))))
"#;

fn module(imports: &str, body: &str) -> Vec<u8> {
    wat::parse_str(format!("(module {imports} {PRELUDE} {body})")).unwrap()
}

fn load(bytes: &[u8], network: bool) -> Arc<dyn LoadedCode> {
    WasmCodeEngine::default().load(bytes, network).unwrap()
}

/// Answers with the arguments it was given, so `{"text": "…"}` comes straight back.
const ECHO: &str = r#"
    (func (export "sovra_call") (param i32 i32 i32 i32) (result i64)
      (call $pack (local.get 2) (local.get 3)))
"#;

#[test]
fn a_tool_call_hands_over_its_arguments_and_returns_the_text() {
    let code = load(&module("", ECHO), false);
    let reply = code.call("echo", &json!({"text": "Tuesday"}), network());
    assert_eq!(reply, Ok("Tuesday".into()));
    let error = code.call("echo", &json!({"error": "No such date."}), network());
    assert_eq!(error, Err("No such date.".into()));
    let odd = code.call("echo", &json!({"something": "else"}), network());
    assert_eq!(odd, Err(BROKEN.into()));
}

#[test]
fn only_sovras_own_imports_are_linked_and_the_network_needs_listed_addresses() {
    let engine = WasmCodeEngine::default();
    let wasi = module(
        r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#,
        ECHO,
    );
    let error = engine.load(&wasi, true).unwrap_err();
    assert!(
        error.0.contains("wasi_snapshot_preview1.fd_write"),
        "{error}"
    );

    let fetching = module(
        r#"(import "sovra" "fetch" (func (param i32 i32) (result i32)))"#,
        ECHO,
    );
    let error = engine.load(&fetching, false).unwrap_err();
    assert!(error.0.contains("lists no addresses"), "{error}");
    assert!(engine.load(&fetching, true).is_ok());

    let memory_import = module(r#"(import "sovra" "log" (memory 1))"#, ECHO);
    assert!(engine.load(&memory_import, false).is_err());

    let no_call = wat::parse_str(format!("(module {PRELUDE})")).unwrap();
    let error = engine.load(&no_call, false).unwrap_err();
    assert!(error.0.contains("sovra_call"), "{error}");
    assert!(engine.load(b"#!/bin/sh", false).is_err());
}

#[test]
fn a_runaway_loop_is_stopped_by_its_instruction_budget() {
    let spin = module(
        "",
        r#"(func (export "sovra_call") (param i32 i32 i32 i32) (result i64)
             (loop $forever (br $forever))
             (i64.const 0))"#,
    );
    let engine = WasmCodeEngine {
        fuel: 5_000_000,
        ..WasmCodeEngine::default()
    };
    let code = engine.load(&spin, false).unwrap();
    let error = code.call("spin", &json!({}), network()).unwrap_err();
    assert!(error.contains("took too long"), "{error}");
}

#[test]
fn memory_cannot_grow_past_the_cap() {
    // Asks for 128 MB; growth fails (returns -1) and the module reports it.
    let greedy = module(
        "",
        r#"(data (i32.const 0) "{\"error\":\"no room\"}")
           (data (i32.const 64) "{\"text\":\"grew\"}")
           (func (export "sovra_call") (param i32 i32 i32 i32) (result i64)
             (if (result i64) (i32.eq (memory.grow (i32.const 2048)) (i32.const -1))
               (then (call $pack (i32.const 0) (i32.const 19)))
               (else (call $pack (i32.const 64) (i32.const 15)))))"#,
    );
    let code = load(&greedy, false);
    assert_eq!(
        code.call("grow", &json!({}), network()),
        Err("no room".into())
    );
}

const FETCHING: &str = r#"(import "sovra" "fetch" (func $fetch (param i32 i32) (result i32)))
    (import "sovra" "take_reply" (func $take (param i32)))"#;

#[test]
fn network_requests_go_through_the_host_and_its_answer_comes_back() {
    let fetch_once = module(
        FETCHING,
        r#"(data (i32.const 0) "{\"url\":\"https://api.example.com/today\"}")
           (func (export "sovra_call") (param i32 i32 i32 i32) (result i64)
             (local $len i32) (local $at i32)
             (local.set $len (call $fetch (i32.const 0) (i32.const 39)))
             (local.set $at (i32.const 8192))
             (call $take (local.get $at))
             (call $pack (local.get $at) (local.get $len)))"#,
    );
    let code = load(&fetch_once, true);
    let host = network();
    let reply = code.call("today", &json!({}), host.clone());
    assert_eq!(reply, Err("blocked for the test".into()));
    assert_eq!(
        *host.asked.lock().unwrap(),
        ["https://api.example.com/today"]
    );
}

#[test]
fn one_answer_cannot_make_endless_requests() {
    let hammer = module(
        FETCHING,
        r#"(data (i32.const 0) "{\"url\":\"https://api.example.com/\"}")
           (func (export "sovra_call") (param i32 i32 i32 i32) (result i64)
             (loop $again
               (drop (call $fetch (i32.const 0) (i32.const 34)))
               (br $again))
             (i64.const 0))"#,
    );
    let code = load(&hammer, true);
    let host = network();
    let error = code.call("hammer", &json!({}), host.clone()).unwrap_err();
    assert!(error.contains("too many internet requests"), "{error}");
    assert_eq!(host.asked.lock().unwrap().len(), MAX_FETCHES_PER_CALL);
}

#[test]
fn every_call_starts_fresh() {
    // Succeeds only on the first call of an instance: state never carries over.
    let counter = module(
        "",
        r#"(global $calls (mut i32) (i32.const 0))
           (data (i32.const 0) "{\"text\":\"first\"}")
           (data (i32.const 64) "{\"error\":\"remembered\"}")
           (func (export "sovra_call") (param i32 i32 i32 i32) (result i64)
             (global.set $calls (i32.add (global.get $calls) (i32.const 1)))
             (if (result i64) (i32.eq (global.get $calls) (i32.const 1))
               (then (call $pack (i32.const 0) (i32.const 16)))
               (else (call $pack (i32.const 64) (i32.const 22)))))"#,
    );
    let code = load(&counter, false);
    for _ in 0..3 {
        assert_eq!(
            code.call("count", &json!({}), network()),
            Ok("first".into())
        );
    }
}

#[test]
fn a_crash_or_an_oversized_answer_is_reported_plainly() {
    let crash = module(
        "",
        r#"(func (export "sovra_call") (param i32 i32 i32 i32) (result i64) unreachable)"#,
    );
    let error = load(&crash, false)
        .call("crash", &json!({}), network())
        .unwrap_err();
    assert!(error.contains("crashed"), "{error}");

    let huge = module(
        "",
        r#"(func (export "sovra_call") (param i32 i32 i32 i32) (result i64)
             (call $pack (i32.const 0) (i32.const 1000000)))"#,
    );
    let error = load(&huge, false)
        .call("huge", &json!({}), network())
        .unwrap_err();
    assert!(error.contains("too long"), "{error}");
}

/// This repository's own plugin, built from `examples/code-plugin`.
const DATE_HELPER: &[u8] = include_bytes!("../../../plugin.wasm");

#[test]
fn the_repositorys_date_helper_runs_in_the_sealed_box() {
    sovra_core::plugins::CodeManifest::parse(include_bytes!("../../../sovra-plugin.json"))
        .unwrap()
        .with_module(DATE_HELPER)
        .unwrap();
    let code = load(DATE_HELPER, false);
    let ask = |tool: &str, arguments: Value| code.call(tool, &arguments, network());
    assert_eq!(
        ask(
            "days_between",
            json!({"from": "2026-11-20", "to": "2026-12-25"})
        ),
        Ok("From 20 November 2026 to 25 December 2026 is 35 days.".into())
    );
    assert_eq!(
        ask(
            "days_between",
            json!({"from": "2024-03-01", "to": "2024-02-28"})
        ),
        Ok("From 1 March 2024 to 28 February 2024 is 2 days earlier.".into())
    );
    assert_eq!(
        ask("weekday", json!({"date": "2000-01-01"})),
        Ok("1 January 2000 is a Saturday.".into())
    );
    assert_eq!(
        ask("add_days", json!({"date": "2026-11-20", "days": 100})),
        Ok("20 November 2026 plus 100 days is 28 February 2027, a Sunday.".into())
    );
    assert_eq!(
        ask("add_days", json!({"date": "2024-03-01", "days": -1})),
        Ok("1 March 2024 plus -1 day is 29 February 2024, a Thursday.".into())
    );
    let error = ask("weekday", json!({"date": "2026-02-30"})).unwrap_err();
    assert!(error.contains("not a real date"), "{error}");
    let error = ask("launch", json!({})).unwrap_err();
    assert!(error.contains("no tool by that name"), "{error}");
}
