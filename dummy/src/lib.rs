use std::ffi::{CStr, CString};
use std::os::raw::c_char;

#[repr(C)]
pub struct PeerConfig {
    semaphore_endpoint: *const c_char,
    panic_on_disconnection: bool,
}

#[derive(Debug, Default)]
pub struct Config {
    state: PeerState,
    semaphore_endpoint: String,
    panic_on_disconnection: bool,
}

#[repr(C)]
pub struct StateHandle {
    _private: [u8; 0], // Zero-sized field
}

#[repr(i32)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum PeerState {
    #[default]
    Created = 0,
    Initialized = 1,
    Running = 2,
    Stopped = 3,
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErrorCode {
    Ok,
    NullHandle,
    InvalidState(PeerState),
}

#[derive(Debug, Default)]
struct State {
    counter: Option<u32>,
    config: Config,
}

#[no_mangle]
pub extern "C" fn peer_create() -> *mut StateHandle {
    let state = Box::new(State::default()); // 1. Allocate State on heap
    Box::into_raw(state) as *mut StateHandle // 2. Convert to raw pointer, cast type
}

#[no_mangle]
pub extern "C" fn peer_init(handle: *mut StateHandle, peer_config: PeerConfig) -> ErrorCode {
    let Ok(state) = get_state(handle) else {
        return ErrorCode::NullHandle;
    };

    println!("Initializing peer with config: {state:?}");

    let current = state.config.state;
    if current != PeerState::Created {
        return ErrorCode::InvalidState(current);
    }

    state.config.state = PeerState::Initialized;
    let c_str = unsafe { CStr::from_ptr(peer_config.semaphore_endpoint) };
    state.config.semaphore_endpoint = c_str.to_str().unwrap_or_default().to_string();
    state.config.panic_on_disconnection = peer_config.panic_on_disconnection;

    ErrorCode::Ok
}

#[no_mangle]
pub extern "C" fn peer_start(handle: *mut StateHandle) -> ErrorCode {
    let Ok(state) = get_state(handle) else {
        return ErrorCode::NullHandle;
    };

    println!("peer_start peer with config: {state:?}");

    let current = state.config.state;
    if current != PeerState::Initialized && current != PeerState::Stopped {
        return ErrorCode::InvalidState(current);
    }

    state.config.state = PeerState::Running;

    ErrorCode::Ok
}

#[no_mangle]
pub extern "C" fn peer_stop(handle: *mut StateHandle) -> ErrorCode {
    let Ok(state) = get_state(handle) else {
        return ErrorCode::NullHandle;
    };

    println!("peer_stop peer with config: {state:?}");

    let current = state.config.state;
    if current != PeerState::Running {
        return ErrorCode::InvalidState(current);
    }

    state.config.state = PeerState::Stopped;

    ErrorCode::Ok
}

#[no_mangle]
pub extern "C" fn peer_get_state(handle: *mut StateHandle, out_state: *mut PeerState) -> ErrorCode {
    let Ok(state) = get_state(handle) else {
        return ErrorCode::NullHandle;
    };

    if !out_state.is_null() {
        unsafe { *out_state = state.config.state; };
        ErrorCode::Ok
    } else {
        ErrorCode::NullHandle
    }

}

#[no_mangle]
pub extern "C" fn peer_get_counter(
    handle: *mut StateHandle,
    out_counter: *mut u32
) -> ErrorCode {
    let Ok(state) = get_state(handle) else {
        return ErrorCode::NullHandle;
    };

    if out_counter.is_null() {
        return ErrorCode::NullHandle;
    }

    match state.counter {
        Some(value) => {
            unsafe { *out_counter = value; }
            ErrorCode::Ok
        }
        None => {
            unsafe { *out_counter = 0; }  // Default value
            ErrorCode::Ok
        }
    }
}

#[no_mangle]
pub extern "C" fn peer_set_counter(
    handle: *mut StateHandle,
    value: u32
) -> ErrorCode {
    let Ok(state) = get_state(handle) else {
        return ErrorCode::NullHandle;
    };

    let current = state.config.state;
    if current != PeerState::Running {
        return ErrorCode::InvalidState(current);
    }

    state.counter = Some(value);

    ErrorCode::Ok
}

fn get_state(handle: *mut StateHandle) -> Result<&'static mut State, ()> {
    if handle.is_null() {
        return Err(());
    }

    // Safety: We ensure that the handle is valid and was created by peer_create
    let state = unsafe { &mut *(handle as *mut State) };
    Ok(state)
}
