#![allow(dead_code)]

pub fn probe_lua() {
    #[cfg(feature = "lua54")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua54);

    #[cfg(feature = "lua53")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua53);

    #[cfg(feature = "lua52")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua52);

    #[cfg(feature = "lua51")]
    let artifacts = lua_src::Build::new().build(lua_src::Lua51);

    #[cfg(feature = "luajit")]
    let artifacts = luajit_src::Build::new()
        .lua52compat(cfg!(feature = "luajit52"))
        .build();

    #[cfg(feature = "luau-lute")]
    {
        #[cfg(not(feature = "luau"))]
        {
            compile_error!("lute runtime requires luau feature to be enabled");
        }

        #[cfg(feature = "luau-vector4")]
        {
            compile_error!("lute runtime does not support vector4 builds");
        }

        #[cfg(feature = "luau-lute-prebuilt")]
        {
            #[cfg(feature = "luau-lute-crypto")]
            {
                compile_error!("Prebuilt lute runtime does not support crypto feature");
            }
            #[cfg(feature = "luau-lute-net")]
            {
                compile_error!("Prebuilt lute runtime does not support net feature");
            }
            #[cfg(feature = "luau-jit")]
            {
                compile_error!("Prebuilt lute runtime does not support native code generation yet (to be supported in the future)");
            }
            
            lute_prebuilts_chooser::integrate();
        } 
    
        #[cfg(not(feature = "luau-lute-prebuilt"))]
        {
            let lcfg = lute_src_rs::LConfig {
                disable_crypto: if cfg!(feature = "luau-lute-crypto") {
                    false
                } else {
                    true
                },
                disable_net: if cfg!(feature = "luau-lute-net") {
                    false
                } else {
                    true
                },
                ..Default::default()
            };
            lute_src_rs::build_lute(lcfg);
        }
    }
    #[cfg(not(feature = "luau-lute"))]
    {
        #[cfg(feature = "luau")]
        let artifacts = luau0_src::Build::new()
            .enable_codegen(cfg!(feature = "luau-codegen"))
            .set_max_cstack_size(1000000)
            .set_vector_size(if cfg!(feature = "luau-vector4") { 4 } else { 3 })
            .build();

        artifacts.print_cargo_metadata();
    }
}
