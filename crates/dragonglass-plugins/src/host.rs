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

use wasmtime::{Engine, Instance, Module, Store};

use crate::{Error, PluginInfo, Result};

/// Execute `code` with `input`, returning the plugin's output.
pub fn run_code(code: &[u8], input: &str) -> Result<String> {
    let engine = Engine::default();
    let module = Module::new(&engine, code)?;

    let mut store = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])?;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or(Error::MissingExport("memory"))?;
    let alloc = instance.get_typed_func::<i32, i32>(&mut store, "alloc")?;
    let run = instance.get_typed_func::<(i32, i32), i64>(&mut store, "run")?;

    let input_bytes = input.as_bytes();
    let pointer = alloc.call(&mut store, input_bytes.len() as i32)?;
    memory.write(&mut store, pointer as usize, input_bytes)?;

    let packed = run.call(&mut store, (pointer, input_bytes.len() as i32))?;
    let out_pointer = ((packed >> 32) & 0xffff_ffff) as usize;
    let out_length = (packed & 0xffff_ffff) as usize;

    let mut output = vec![0u8; out_length];
    memory.read(&store, out_pointer, &mut output)?;

    Ok(String::from_utf8_lossy(&output).into_owned())
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
}
