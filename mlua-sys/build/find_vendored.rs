#![allow(dead_code)]

pub fn probe_lua() {
    let artifacts = luau0_src::Build::new()
        .enable_codegen(cfg!(feature = "luau-codegen"))
        .set_max_cstack_size(1000000)
        .set_vector_size(if cfg!(feature = "luau-vector4") { 4 } else { 3 })
        .build();

    artifacts.print_cargo_metadata();
}
