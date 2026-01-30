# Rust FFI Example

This project demonstrates how to call Rust code from C using FFI (Foreign Function Interface).

## Structure

- `src/lib.rs` - Rust library with FFI-exported functions
- `ffi_example.c` - C program that calls the Rust functions
- `Cargo.toml` - Configured to build a C-compatible library (both static and dynamic)
- `flake.nix` - Nix flake that builds everything

## Available Commands

### Build and run the FFI example
```bash
nix run
# or explicitly:
nix run .#ffi
```

### Build specific packages
```bash
# Build just the Rust library
nix build .#rust-lib

# Build the FFI example (C calling Rust)
nix build .#ffi-example

# Build the original dummy C program
nix build .#dummy
```

### Run different apps
```bash
# Run the FFI example (default)
nix run

# Run the original dummy app
nix run .#dummy
```

## How it works

1. **Rust Library** (`src/lib.rs`):
   - Uses `#[no_mangle]` to prevent name mangling
   - Uses `extern "C"` to use C calling convention
   - Configured in `Cargo.toml` with `crate-type = ["cdylib", "staticlib"]`

2. **C Program** (`ffi_example.c`):
   - Declares Rust functions with `extern`
   - Links against the Rust library during compilation

3. **Nix Flake**:
   - `rustLib` package builds the Rust library
   - `ffiExample` package compiles C code and links with Rust library
   - Static linking is used by default for simplicity

## Example Functions

The Rust library exports:
- `add_numbers(a, b)` - Simple addition
- `greet(name)` - Returns a greeting string (demonstrates string handling)
- `free_rust_string(s)` - Frees strings allocated by Rust
- `factorial(n)` - Computes factorial

## Development

To experiment:
1. Add new functions to `src/lib.rs` with `#[no_mangle]` and `extern "C"`
2. Declare them in `ffi_example.c` with `extern`
3. Call them from C code
4. Run with `nix run`

## Important Notes

- Always use `#[no_mangle]` and `extern "C"` for FFI functions
- Be careful with memory management across FFI boundaries
- Strings allocated in Rust must be freed in Rust (use `free_rust_string`)
- The library is statically linked by default
