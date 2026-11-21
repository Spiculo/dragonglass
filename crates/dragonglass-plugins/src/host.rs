//! Running plugin commands inside a sandboxed wasmtime instance.
//!
//! A plugin is a WebAssembly module exporting:
//!
//! ```text
//! memory                  linear memory
//! alloc(len: i32) -> i32  allocate `len` bytes, returning a pointer
//! run(ptr: i32, len: i32) -> i64
//!                         run the command; returns (out_ptr << 32) | out_len
//! ```
//!
//! The host writes the command request as JSON at `ptr` and reads the JSON response from
//! the returned region. Any language that compiles to `wasm32` can implement this small
//! ABI; no WASI, filesystem or network access is provided, so plugins are inert unless the
//! host hands them something.

use std::path::Path;

use wasmtime::{Config, Engine, Instance, Module, Store, StoreLimitsBuilder};

use crate::{Error, PluginInfo, Result};

const FUEL: u64 = 20_000_000;
const MAX_MEMORY: usize = 64 * 1024 * 1024;
const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// Execute `code` with `input`, returning the plugin's output.
pub fn run_code(code: &[u8], input: &str) -> Result<String> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.max_wasm_stack(512 * 1024);
    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, code)?;

    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_MEMORY)
        .table_elements(10_000)
        .instances(10)
        .build();
    let mut store = Store::new(&engine, limits);
    store.limiter(|limits| limits);
    store.set_fuel(FUEL)?;
    let instance = Instance::new(&mut store, &module, &[])?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or(Error::MissingExport("memory"))?;
    let alloc = instance.get_typed_func::<i32, i32>(&mut store, "alloc")?;
    let run = instance.get_typed_func::<(i32, i32), i64>(&mut store, "run")?;

    let input_bytes = input.as_bytes();
    let input_len = i32::try_from(input_bytes.len())
        .map_err(|_| Error::InvalidRequest("plugin input is too large".to_string()))?;
    let pointer = alloc.call(&mut store, input_len)?;
    if pointer < 0
        || memory.data_size(&store) < (pointer as usize).saturating_add(input_bytes.len())
    {
        return Err(Error::InvalidRequest(
            "plugin alloc returned an out-of-bounds pointer".to_string(),
        ));
    }
    memory.write(&mut store, pointer as usize, input_bytes)?;

    let packed = run.call(&mut store, (pointer, input_len))?;
    let out_pointer = ((packed >> 32) & 0xffff_ffff) as usize;
    let out_length = (packed & 0xffff_ffff) as usize;

    // Never trust the plugin's length: validate before allocating or copying.
    let size = memory.data_size(&store);
    if out_length > MAX_OUTPUT
        || out_pointer > size
        || out_length > size.saturating_sub(out_pointer)
    {
        return Err(Error::InvalidRequest(
            "plugin returned an out-of-bounds output".to_string(),
        ));
    }
    let bytes = memory.data(&store)[out_pointer..out_pointer + out_length].to_vec();

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Load a plugin from disk and run one of its commands.
pub fn run_command(
    plugin: &PluginInfo,
    command: &str,
    vault_root: &Path,
    note: Option<&str>,
) -> Result<String> {
    let code = std::fs::read(plugin.wasm_path())?;
    let request = serde_json::json!({
        "command": command,
        "vault": vault_root.to_string_lossy(),
        "note": note,
    });

    run_code(&code, &request.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Echoes the request straight back.
    const ECHO: &str = r#"
(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))
  (func (export "alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $ptr))
  (func (export "run") (param $ptr i32) (param $len i32) (result i64)
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
      (i64.extend_i32_u (local.get $len)))))
"#;

    /// Always answers with a fixed message.
    const PONG: &str = r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 0) "pong")
  (func (export "alloc") (param i32) (result i32) (i32.const 64))
  (func (export "run") (param i32) (param i32) (result i64)
    (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const 4))))
"#;

    #[test]
    fn echoes_the_request_back() {
        let output = run_code(ECHO.as_bytes(), "hello plugin").unwrap();
        assert_eq!(output, "hello plugin");
    }

    #[test]
    fn returns_a_fixed_message() {
        assert_eq!(run_code(PONG.as_bytes(), "ignored").unwrap(), "pong");
    }

    #[test]
    fn missing_exports_are_reported() {
        let incomplete = "(module (memory (export \"memory\") 1))";
        assert!(run_code(incomplete.as_bytes(), "").is_err());
    }

    #[test]
    fn oversized_output_is_rejected_without_allocating_it() {
        const HUGE: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) (i32.const 64))
  (func (export "run") (param i32) (param i32) (result i64)
    (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const 0xffffffff))))
"#;
        assert!(run_code(HUGE.as_bytes(), "").is_err());
    }

    #[test]
    fn infinite_plugins_run_out_of_fuel() {
        const SPIN: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "alloc") (param i32) (result i32) (i32.const 64))
  (func (export "run") (param i32) (param i32) (result i64)
    (loop $again (br $again))
    (i64.const 0)))
"#;
        assert!(run_code(SPIN.as_bytes(), "").is_err());
    }
}
