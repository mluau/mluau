use mluau::{Lua, Result};

#[test]
fn test_debug_format() -> Result<()> {
    let lua = Lua::new();

    // Globals
    let globals = lua.globals();
    let dump = format!("{globals:#?}");
    assert!(dump.starts_with("{\n  _G = table:"));

    // TODO: Other cases

    Ok(())
}

#[test]
fn test_traceback() -> Result<()> {
    let lua = Lua::new_with(
        mluau::StdLib::ALL_SAFE,
        mluau::LuaOptions::new().disable_error_userdata(true),
    )?;

    let tracebacker = lua.create_function(|lua, _: ()| {
        let tb1 = lua.traceback()?;
        let tbth = lua.current_thread().traceback()?;
        assert_eq!(tb1, tbth);
        Ok(tb1)
    })?;

    let chunk = lua
        .load("local a = ...; return a()")
        .set_name("mychunk")
        .into_function()?
        .call::<String>(tracebacker)?;

    #[cfg(feature = "luau")]
    assert!(chunk.contains("string \"mychunk\""));

    Ok(())
}
