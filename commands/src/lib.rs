#[allow(clippy::all, clippy::pedantic, warnings)]
pub mod common_capnp {
    include!(concat!(env!("OUT_DIR"), "/common_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, warnings)]
pub mod master_capnp {
    include!(concat!(env!("OUT_DIR"), "/master_capnp.rs"));
}

#[allow(clippy::all, clippy::pedantic, warnings)]
pub mod worker_capnp {
    include!(concat!(env!("OUT_DIR"), "/worker_capnp.rs"));
}
