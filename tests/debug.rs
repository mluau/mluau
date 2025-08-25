use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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

#[test]
#[cfg(all(feature = "luau"))]
fn test_gc_state_name() -> Result<()> {
    let lua = Lua::new();

    assert_eq!(lua.gc_state_name(0), Some("pause".to_string()));
    assert_eq!(lua.gc_state_name(1), Some("mark".to_string()));
    assert_eq!(lua.gc_state_name(2), Some("remark".to_string()));
    assert_eq!(lua.gc_state_name(3), Some("atomic".to_string()));
    assert_eq!(lua.gc_state_name(4), Some("sweep".to_string()));
    assert_eq!(lua.gc_state_name(5), None);

    Ok(())
}

#[test]
#[cfg(feature = "luau")]
fn test_gc_allocation_rate() -> Result<()> {
    let lua = Lua::new();

    let _ = lua.gc_allocation_rate();

    Ok(())
}

#[test]
#[cfg(feature = "luau")]
fn test_gc_interrupt() -> Result<()> {
    let lua = Lua::new_with(
        mluau::StdLib::ALL_SAFE,
        mluau::LuaOptions::new().disable_error_userdata(true),
    )?;

    lua.set_interrupt(|_lua| Ok(mluau::VmState::Continue));

    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupted_clone = interrupted.clone();
    lua.set_gc_interrupt(move |lua, gc_state| {
        interrupted_clone.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    // Allocate a lot of memory to trigger GC
    let tbl: mluau::Table = lua
        .load("local t={}; for i=1,1e5 do t[i]={} end; return t")
        .eval()?;
    drop(tbl);
    lua.gc_collect()?;
    assert!(interrupted.load(std::sync::atomic::Ordering::SeqCst));

    Ok(())
}
