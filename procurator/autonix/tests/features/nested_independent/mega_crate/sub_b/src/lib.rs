pub fn sub_b_func() -> &'static str {
    sub_a::sub_a_func();
    "From sub_b in nested workspace"
}
