use mluau::prelude::*;

fn sum(_: &Lua, (a, b): (i64, i64)) -> LuaResult<i64> {
    Ok(a + b)
}

fn used_memory(lua: &Lua, _: ()) -> LuaResult<usize> {
    Ok(lua.used_memory())
}

fn check_userdata(_: &Lua, ud: LuaAnyUserData) -> LuaResult<i32> {
    Ok(ud.borrow::<MyUserData>()?.0)
}

#[mluau::lua_module]
fn test_module(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set("sum", lua.create_function(sum)?)?;
    exports.set("used_memory", lua.create_function(used_memory)?)?;
    exports.set("check_userdata", lua.create_function(check_userdata)?)?;
    Ok(exports)
}

#[derive(Clone, Copy)]
struct MyUserData(i32);

impl LuaUserData for MyUserData {}

#[mluau::lua_module(name = "test_module_second", skip_memory_check)]
fn test_module2(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set("userdata", MyUserData(123))?;
    Ok(exports)
}

#[mluau::lua_module]
fn test_module_new_vm(lua: &Lua) -> LuaResult<LuaTable> {
    let eval = lua.create_function(|_, prog: String| {
        let lua = Lua::new();
        lua.load(prog).eval::<Option<String>>()
    })?;

    let exports = lua.create_table()?;
    exports.set("eval", eval)?;
    Ok(exports)
}

#[mluau::lua_module]
fn test_module_error(_: &Lua) -> LuaResult<LuaTable> {
    Err("custom module error".into_lua_err())
}
