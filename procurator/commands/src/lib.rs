pub mod common_capnp {
    include!(concat!(env!("OUT_DIR"), "/common_capnp.rs"));
}

pub mod master_capnp {
    include!(concat!(env!("OUT_DIR"), "/master_capnp.rs"));
}

pub mod worker_capnp {
    include!(concat!(env!("OUT_DIR"), "/worker_capnp.rs"));
}
