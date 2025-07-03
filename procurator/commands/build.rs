fn main() {
    ::capnpc::CompilerCommand::new()
        .src_prefix("schema")
        .file("schema/commands.capnp")
        .run()
        .expect("compiling schema");
}
