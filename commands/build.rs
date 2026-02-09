fn main() {
    ::capnpc::CompilerCommand::new()
        .src_prefix("schema")
        .file("schema/common.capnp")
        .file("schema/master.capnp")
        .file("schema/worker.capnp")
        .run()
        .expect("compiling schema");
}
