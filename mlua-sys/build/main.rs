mod luau_build;

fn main() {
    let artifacts = luau_build::Build::new()
        .enable_codegen(cfg!(feature = "luau-codegen"))
        .set_vector_size(if cfg!(feature = "luau-vector4") { 4 } else { 3 })
        .build();

    artifacts.print_cargo_metadata();
}
