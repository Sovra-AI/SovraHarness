//! The sealed box plugin code runs in: the core `PluginCodeEngine` port on the
//! `wasmi` WebAssembly interpreter.
//!
//! A plugin module sees only what this crate links in: `sovra.log` always, and
//! `sovra.fetch` / `sovra.take_reply` when its plugin lists internet addresses (the
//! runtime's `CodeHost` then checks every request against that list). There is no
//! WASI, so no files, clock, environment, randomness or programs. Every tool call
//! gets a fresh instance with an instruction budget and a memory cap, so a stuck
//! or greedy module stops instead of holding the computer, and nothing it does in
//! one call survives into the next.
//!
//! The module's side of the contract (documented in `docs/plugins.md`):
//!
//! - export `memory`, `sovra_alloc(len: i32) -> i32` and
//!   `sovra_call(name_ptr, name_len, args_ptr, args_len: i32) -> i64`;
//! - `sovra_call` receives the tool name and its JSON arguments in memory it
//!   allocated, and returns `(ptr << 32) | len` of a UTF-8 JSON reply,
//!   `{"text": "…"}` or `{"error": "…"}`;
//! - `fetch(ptr, len: i32) -> i32` takes a JSON request (`url`, `method`,
//!   `headers`, `body`) and returns the length of the JSON reply
//!   (`{"status", "body"}` or `{"error"}`), which `take_reply(ptr: i32)` copies into
//!   memory the module allocated.

use std::sync::Arc;

use serde_json::{Value, json};
use sovra_core::plugins::{
    CodeFetch, CodeHost, LoadedCode, MAX_FETCHES_PER_CALL, MAX_MODULE_BYTES, PluginCodeEngine,
};
use sovra_core::skills::SkillError;
use wasmi::{
    Caller, Config, Engine, Extern, ExternType, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, TrapCode, TypedFunc,
};

/// Instructions one tool call may run: a few seconds of work in the interpreter.
const FUEL_PER_CALL: u64 = 2_000_000_000;
/// Linear memory one call may grow to.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
/// Largest tool name plus arguments handed in, and largest reply taken back.
const MAX_INPUT_BYTES: usize = 256 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_LOG_LINES: usize = 32;
const MAX_LOG_BYTES: usize = 300;
const IMPORT_MODULE: &str = "sovra";

/// The sandbox. Cheap to share; each loaded module keeps its own engine handle.
#[derive(Debug, Clone)]
pub struct WasmCodeEngine {
    engine: Engine,
    fuel: u64,
}

impl Default for WasmCodeEngine {
    fn default() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        Self {
            engine: Engine::new(&config),
            fuel: FUEL_PER_CALL,
        }
    }
}

impl PluginCodeEngine for WasmCodeEngine {
    fn load(&self, module: &[u8], network: bool) -> Result<Arc<dyn LoadedCode>, SkillError> {
        if module.len() > MAX_MODULE_BYTES {
            return Err(refused("it is larger than 2 MB"));
        }
        let module = Module::new(&self.engine, module).map_err(|problem| {
            refused(&format!("it is not a valid WebAssembly module ({problem})"))
        })?;
        for import in module.imports() {
            let allowed = import.module() == IMPORT_MODULE
                && match import.name() {
                    "log" => true,
                    "fetch" | "take_reply" => network,
                    _ => false,
                };
            if !allowed || !matches!(import.ty(), ExternType::Func(_)) {
                let wanted = format!("{}.{}", import.module(), import.name());
                return Err(refused(&if import.name() == "fetch" && !network {
                    "it asks for the internet but its sovra-plugin.json lists no addresses".into()
                } else {
                    format!("it asks for \"{wanted}\", which Sovra does not give plugin code")
                }));
            }
        }
        for (name, wanted) in [
            ("memory", "memory"),
            ("sovra_alloc", "func"),
            ("sovra_call", "func"),
        ] {
            let present = matches!(
                (module.get_export(name), wanted),
                (Some(ExternType::Memory(_)), "memory") | (Some(ExternType::Func(_)), "func")
            );
            if !present {
                return Err(refused(&format!("it does not export \"{name}\"")));
            }
        }
        let mut linker = Linker::<Guest>::new(&self.engine);
        link(&mut linker, network).map_err(|problem| {
            refused(&format!("the sandbox could not be prepared ({problem})"))
        })?;
        Ok(Arc::new(WasmCode {
            engine: self.engine.clone(),
            fuel: self.fuel,
            module,
            linker: Arc::new(linker),
        }))
    }
}

fn refused(reason: &str) -> SkillError {
    SkillError(format!("The plugin's code cannot run: {reason}."))
}

/// What one call's store holds: its limits, the network gate and the pending reply.
struct Guest {
    limits: StoreLimits,
    host: Arc<dyn CodeHost>,
    fetches: usize,
    reply: Option<Vec<u8>>,
    logs: usize,
}

#[derive(Debug)]
struct WasmCode {
    engine: Engine,
    fuel: u64,
    module: Module,
    linker: Arc<Linker<Guest>>,
}

impl std::fmt::Debug for Guest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Guest")
    }
}

impl LoadedCode for WasmCode {
    fn call(
        &self,
        tool: &str,
        arguments: &Value,
        host: Arc<dyn CodeHost>,
    ) -> Result<String, String> {
        let arguments = serde_json::to_vec(arguments).map_err(|_| BROKEN.to_owned())?;
        if tool.len() + arguments.len() > MAX_INPUT_BYTES {
            return Err("The request for this tool is too large.".into());
        }
        let mut store = Store::new(
            &self.engine,
            Guest {
                limits: StoreLimitsBuilder::new()
                    .memory_size(MEMORY_LIMIT)
                    .memories(1)
                    .tables(4)
                    .instances(1)
                    .build(),
                host,
                fetches: 0,
                reply: None,
                logs: 0,
            },
        );
        store.limiter(|guest| &mut guest.limits);
        store.set_fuel(self.fuel).map_err(|_| BROKEN.to_owned())?;
        let instance = self
            .linker
            .instantiate_and_start(&mut store, &self.module)
            .map_err(|problem| explain(&problem))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| BROKEN.to_owned())?;
        let alloc: TypedFunc<i32, i32> = instance
            .get_typed_func(&store, "sovra_alloc")
            .map_err(|_| BROKEN.to_owned())?;
        let call: TypedFunc<(i32, i32, i32, i32), i64> = instance
            .get_typed_func(&store, "sovra_call")
            .map_err(|_| BROKEN.to_owned())?;
        let (name_ptr, name_len) = place(&mut store, alloc, memory, tool.as_bytes())?;
        let (args_ptr, args_len) = place(&mut store, alloc, memory, &arguments)?;
        let packed = call
            .call(&mut store, (name_ptr, name_len, args_ptr, args_len))
            .map_err(|problem| explain(&problem))?;
        let packed = u64::from_ne_bytes(packed.to_ne_bytes());
        let (pointer, length) = (packed >> 32, packed & 0xffff_ffff);
        let length = usize::try_from(length).map_err(|_| BROKEN.to_owned())?;
        if length > MAX_OUTPUT_BYTES {
            return Err("The plugin's answer was too long.".into());
        }
        let mut reply = vec![0; length];
        memory
            .read(
                &store,
                usize::try_from(pointer).map_err(|_| BROKEN.to_owned())?,
                &mut reply,
            )
            .map_err(|_| BROKEN.to_owned())?;
        answer(&reply)
    }
}

const BROKEN: &str = "The plugin's code did not follow Sovra's plugin rules.";

/// The module's reply as text for the helper, or its own error sentence.
fn answer(reply: &[u8]) -> Result<String, String> {
    let value: Value = serde_json::from_slice(reply).map_err(|_| BROKEN.to_owned())?;
    match (
        value.get("text").and_then(Value::as_str),
        value.get("error").and_then(Value::as_str),
    ) {
        (Some(text), None) => Ok(text.to_owned()),
        (None, Some(error)) => Err(error.chars().take(1000).collect()),
        _ => Err(BROKEN.to_owned()),
    }
}

/// Plain words for a stopped call. Details stay in the log.
fn explain(problem: &wasmi::Error) -> String {
    tracing::debug!(%problem, "plugin code stopped");
    match problem.as_trap_code() {
        Some(TrapCode::OutOfFuel) => "The plugin's code took too long and was stopped.".into(),
        Some(TrapCode::GrowthOperationLimited) => {
            "The plugin's code needed more memory than Sovra allows and was stopped.".into()
        }
        Some(_) => "The plugin's code crashed. Try again, or tell its maker.".into(),
        // A host function's own refusal (a blocked address, too many requests).
        None => problem.to_string(),
    }
}

/// Copies `bytes` into memory the module allocated for them.
fn place(
    store: &mut Store<Guest>,
    alloc: TypedFunc<i32, i32>,
    memory: Memory,
    bytes: &[u8],
) -> Result<(i32, i32), String> {
    let length = i32::try_from(bytes.len()).map_err(|_| BROKEN.to_owned())?;
    let pointer = alloc
        .call(&mut *store, length)
        .map_err(|problem| explain(&problem))?;
    memory
        .write(
            &mut *store,
            usize::try_from(pointer).map_err(|_| BROKEN.to_owned())?,
            bytes,
        )
        .map_err(|_| BROKEN.to_owned())?;
    Ok((pointer, length))
}

fn read_guest(
    caller: &Caller<'_, Guest>,
    pointer: i32,
    length: i32,
) -> Result<Vec<u8>, wasmi::Error> {
    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
        return Err(wasmi::Error::new(BROKEN));
    };
    let (Ok(pointer), Ok(length)) = (usize::try_from(pointer), usize::try_from(length)) else {
        return Err(wasmi::Error::new(BROKEN));
    };
    if length > MAX_INPUT_BYTES {
        return Err(wasmi::Error::new("The plugin's request is too large."));
    }
    let mut bytes = vec![0; length];
    memory
        .read(caller, pointer, &mut bytes)
        .map_err(|_| wasmi::Error::new(BROKEN))?;
    Ok(bytes)
}

fn link(linker: &mut Linker<Guest>, network: bool) -> Result<(), wasmi::Error> {
    linker.func_wrap(
        IMPORT_MODULE,
        "log",
        |mut caller: Caller<'_, Guest>, pointer: i32, length: i32| -> Result<(), wasmi::Error> {
            if caller.data().logs >= MAX_LOG_LINES {
                return Ok(());
            }
            caller.data_mut().logs += 1;
            let length = length.clamp(0, i32::try_from(MAX_LOG_BYTES).unwrap_or(i32::MAX));
            let line = read_guest(&caller, pointer, length)?;
            tracing::debug!(line = %String::from_utf8_lossy(&line), "plugin code");
            Ok(())
        },
    )?;
    if !network {
        return Ok(());
    }
    linker.func_wrap(
        IMPORT_MODULE,
        "fetch",
        |mut caller: Caller<'_, Guest>, pointer: i32, length: i32| -> Result<i32, wasmi::Error> {
            if caller.data().fetches >= MAX_FETCHES_PER_CALL {
                return Err(wasmi::Error::new(
                    "The plugin made too many internet requests for one answer.",
                ));
            }
            caller.data_mut().fetches += 1;
            let request = read_guest(&caller, pointer, length)?;
            let reply = match serde_json::from_slice::<CodeFetch>(&request) {
                Ok(request) => match caller.data().host.fetch(request) {
                    Ok(reply) => json!({"status": reply.status, "body": reply.body}),
                    Err(error) => json!({"error": error}),
                },
                Err(_) => json!({"error": "The request was not in the form Sovra expects."}),
            };
            let reply = serde_json::to_vec(&reply).map_err(|_| wasmi::Error::new(BROKEN))?;
            let length = i32::try_from(reply.len()).map_err(|_| wasmi::Error::new(BROKEN))?;
            caller.data_mut().reply = Some(reply);
            Ok(length)
        },
    )?;
    linker.func_wrap(
        IMPORT_MODULE,
        "take_reply",
        |mut caller: Caller<'_, Guest>, pointer: i32| -> Result<(), wasmi::Error> {
            let reply = caller
                .data_mut()
                .reply
                .take()
                .ok_or_else(|| wasmi::Error::new(BROKEN))?;
            let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
                return Err(wasmi::Error::new(BROKEN));
            };
            let pointer = usize::try_from(pointer).map_err(|_| wasmi::Error::new(BROKEN))?;
            memory
                .write(&mut caller, pointer, &reply)
                .map_err(|_| wasmi::Error::new(BROKEN))
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
