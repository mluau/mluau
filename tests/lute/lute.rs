#![cfg(feature = "luau-lute")]

use mlua::prelude::*;

#[test]
fn test_lute_runtime() -> LuaResult<()> {
    let lua = Lua::new();

    pub struct B {
        v: i32,
    }

    impl LuaUserData for B {
        fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
            fields.add_field_method_get("value", |_, this| Ok(this.v));
        }
    }

    // Register the B type in Luau
    lua.load("b = ...; return b")
        .call::<LuaUserDataRef<B>>(B { v: 42 })?;
    assert_eq!(lua.load("return b.value").eval::<i32>()?, 42);

    // Load the lute runtime
    lua.setup_lute_runtime()?;
    lua.load_lute_stdlib(LuaLuteStdLib::TIME)?;

    pub struct A {
        v: i32,
    }

    impl LuaUserData for A {
        fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
            fields.add_field_method_get("value", |_, this| Ok(this.v));
        }
    }

    // Register the A type in Luau
    lua.load("a = ...; return a")
        .call::<LuaUserDataRef<A>>(A { v: 100 })?;
    assert_eq!(lua.load("return a.value").eval::<i32>()?, 100);

    let res = lua.load("aud = ...; assert(aud.value == 32, 'aud is invalid'); a = time.duration.seconds(2) + time.duration.seconds(3); return a").call::<LuaAnyUserData>(A {
        v: 32
    })?;

    unsafe {
        // Check for lute's special metatable
        let metatable = res.underlying_metatable()?;
        assert_eq!(
            metatable.get::<LuaString>("__metatable")?.to_str()?,
            "The metatable is locked"
        );
    }

    Ok(())
}
