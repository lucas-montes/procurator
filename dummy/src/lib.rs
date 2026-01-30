use std::ffi::CStr;
use std::os::raw::c_char;

pub struct Config {
    state: PeerState,
    semaphore_endpoint: Option<String>,
    panic_on_disconnection: bool,
}

struct State {
    counter: u32,
    config: Config,
}

#[repr(C)]
pub struct PeerHandle {
    _private: [u8; 0], // Zero-sized field
}

enum PeerState {
    Created,
    Initialized,
    Running,
    Stopped,
}

// // Step 1: Create
// let state = Box::new(State::new());
// // Memory: Box<State> at address 0x1000

// let raw_state_ptr = Box::into_raw(state);
// // Memory: *mut State pointing to 0x1000
// // Box is gone, we manually manage memory now

// let opaque_ptr = raw_state_ptr as *mut PeerHandle;
// // Memory: *mut PeerHandle pointing to 0x1000
// // Same address, different type!
// // Return to C/JNI

// // Step 2: Use
// let handle: *mut PeerHandle = /* ... from C */;  // 0x1000
// let state_ptr = handle as *mut State;             // 0x1000 (cast back)
// let state_ref = unsafe { &*state_ptr };           // &State (dereference)
// Now we can use state_ref.config, state_ref.daemon, etc.

pub fn peer_create() -> *mut PeerHandle {
    let state = Box::new(State::new()); // 1. Allocate State on heap
    Box::into_raw(state) as *mut PeerHandle // 2. Convert to raw pointer, cast type
}
pub fn peer_init(handle: *mut PeerHandle, peer_config: PeerConfig) -> Result<(), InitError> {
    let state = get_state(handle)?;

    let current = state.get_state();
    if current != PeerState::Created && current != PeerState::Stopped {
        return Err(InitError::InvalidState {
            expected: PeerState::Created,
            current,
        });
    }

    let mut config = state.config.lock().map_err(|_| InitError::MutexPoisoned)?;

    // ...rest of code...
}

pub fn peer_start(handle: *mut PeerHandle) -> Result<(), InitError> {
    let state = get_state(handle)?;

    let current = state.get_state();
    // ...rest of code...
}

pub fn peer_stop(handle: *mut PeerHandle) -> Result<(), InitError> {
    let state = get_state(handle)?;

    state.check_state(PeerState::Running)?;
    // ...rest of code...
}

/// Add two numbers - simple FFI example
#[no_mangle]
pub extern "C" fn add_numbers(a: i32, b: i32) -> i32 {
    a + b
}

/// Greet someone by name
#[no_mangle]
pub extern "C" fn greet(name: *const c_char) -> *mut c_char {
    if name.is_null() {
        return std::ptr::null_mut();
    }

    let c_str = unsafe { CStr::from_ptr(name) };
    let name_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let greeting = format!("Hello from Rust, {}!", name_str);
    let c_greeting = std::ffi::CString::new(greeting).unwrap();
    c_greeting.into_raw()
}

/// Free a string allocated by Rust
#[no_mangle]
pub extern "C" fn free_rust_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = std::ffi::CString::from_raw(s);
        }
    }
}

/// Compute factorial
#[no_mangle]
pub extern "C" fn factorial(n: u32) -> u64 {
    if n == 0 || n == 1 {
        1
    } else {
        (1..=n as u64).product()
    }
}
