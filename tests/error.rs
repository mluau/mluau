use std::io;

use mluau::{Error, ErrorContext, Lua, Result};

#[test]
fn test_error_context() -> Result<()> {
    let lua = Lua::new();

    let func =
        lua.create_function(|_, ()| Err::<(), _>(Error::runtime("runtime error")).context("some context"))?;
    lua.globals().set("func", func)?;

    let msg = lua
        .load("local _, err = pcall(func); return tostring(err)")
        .eval::<String>()?;
    assert!(msg.contains("some context"));
    assert!(msg.contains("runtime error"));

    let func2 = lua.create_function(|lua, ()| {
        lua.globals()
            .get::<String>("nonextant")
            .with_context(|_| "failed to find global")
    })?;
    lua.globals().set("func2", func2)?;

    let msg2 = lua
        .load("local _, err = pcall(func2); return tostring(err)")
        .eval::<String>()?;
    assert!(msg2.contains("failed to find global"));
    assert!(msg2.contains("error converting Lua nil to String"));

    // Rewrite context message and test `downcast_ref`
    let func3 = lua.create_function(|_, ()| {
        Err::<(), _>(Error::external(io::Error::new(io::ErrorKind::Other, "other")))
            .context("some context")
            .context("some new context")
    })?;
    let err = func3.call::<()>(()).unwrap_err();
    assert!(!err.to_string().contains("some context"));
    assert!(err.to_string().contains("some new context"));

    Ok(())
}

#[test]
fn test_disable_error_userdata() -> Result<()> {
    let lua = Lua::new_with(
        mluau::StdLib::ALL_SAFE,
    )?;

    let func =
        lua.create_function(|_, ()| Err::<(), _>(Error::runtime("runtime error")).context("some context"))?;
    lua.globals().set("func", func)?;

    let msg = lua
        .load("local _, err = pcall(func); return tostring(err)")
        .eval::<String>()?;
    assert!(msg.contains("some context"));
    assert!(msg.contains("runtime error"));

    let func2 = lua.create_function(|lua, ()| {
        lua.globals()
            .get::<String>("nonextant")
            .with_context(|_| "failed to find global")
    })?;
    lua.globals().set("func2", func2)?;

    let msg2 = lua
        .load("local _, err = pcall(func2); return tostring(err)")
        .eval::<String>()?;
    assert!(msg2.contains("failed to find global"));
    assert!(msg2.contains("error converting Lua nil to String"));

    // Rewrite context message and test `downcast_ref`
    let func3 = lua.create_function(|_, ()| {
        Err::<(), _>(Error::external(io::Error::new(io::ErrorKind::Other, "other")))
            .context("some context")
            .context("some new context")
    })?;
    let err = func3.call::<()>(()).unwrap_err();
    assert!(!err.to_string().contains("some context"));
    assert!(err.to_string().contains("some new context"));

    lua.set_memory_limit(1000)?;

    // Force a memory error
    for i in 0..10000 {
        match lua.load(format!("return string.rep('a', {})", i)).exec() {
            Ok(_) => {}
            Err(mluau::Error::MemoryError { .. }) => {
                // Memory error is expected, we can stop here
                break;
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    lua.set_memory_limit(10000000)
        .expect("Failed to set memory limit");

    // Next, test panic handling
    let func4 = lua.create_function(|_, ()| {
        if true {
            panic!("This is a test panic")
        } else {
            Ok(())
        }
    })?;
    lua.globals().set("func4", func4)?;
    let msg4 = lua
        .load("local ok, err = pcall(func4); return tostring(err)")
        .eval::<String>()?;
    assert!(msg4.contains("This is a test panic"));

    let res = lua.globals().get::<mluau::Function>("func4")?.call::<()>(());

    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("This is a test panic"));

    Ok(())
}
