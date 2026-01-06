use std::panic::catch_unwind;

use mluau::{Error, Function, IntoLua, Lua, Result, Thread, ThreadStatus, Value};

#[test]
fn test_thread() -> Result<()> {
    let lua = Lua::new();

    let thread = lua.create_thread(
        lua.load(
            r#"
            function (s)
                local sum = s
                for i = 1,4 do
                    sum = sum + coroutine.yield(sum)
                end
                return sum
            end
            "#,
        )
        .eval()?,
    )?;

    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(0)?, 0);
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(1)?, 1);
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(2)?, 3);
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(3)?, 6);
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(4)?, 10);
    assert_eq!(thread.status(), ThreadStatus::Finished);

    let accumulate = lua.create_thread(
        lua.load(
            r#"
            function (sum)
                while true do
                    sum = sum + coroutine.yield(sum)
                end
            end
            "#,
        )
        .eval::<Function>()?,
    )?;

    for i in 0..4 {
        accumulate.resume::<()>(i)?;
    }
    assert_eq!(accumulate.resume::<i64>(4)?, 10);
    assert_eq!(accumulate.status(), ThreadStatus::Resumable);
    assert!(accumulate.resume::<()>("error").is_err());
    assert_eq!(accumulate.status(), ThreadStatus::Error);

    let thread = lua
        .load(
            r#"
            coroutine.create(function ()
                while true do
                    coroutine.yield(42)
                end
            end)
        "#,
        )
        .eval::<Thread>()?;
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    assert_eq!(thread.resume::<i64>(())?, 42);

    let thread: Thread = lua
        .load(
            r#"
            coroutine.create(function(arg)
                assert(arg == 42)
                local yieldarg = coroutine.yield(123)
                assert(yieldarg == 43)
                return 987
            end)
        "#,
        )
        .eval()?;

    assert_eq!(thread.resume::<u32>(42)?, 123);
    assert_eq!(thread.resume::<u32>(43)?, 987);

    match thread.resume::<u32>(()) {
        Err(Error::CoroutineUnresumable) => {}
        Err(_) => panic!("resuming dead coroutine error is not CoroutineInactive kind"),
        _ => panic!("resuming dead coroutine did not return error"),
    }

    // Already running thread must be unresumable
    let thread = lua.create_thread(lua.create_function(|lua, ()| {
        assert_eq!(lua.current_thread().status(), ThreadStatus::Running);
        let result = lua.current_thread().resume::<()>(());
        assert!(
            matches!(result, Err(Error::CoroutineUnresumable)),
            "unexpected result: {result:?}",
        );
        Ok(())
    })?)?;
    let result = thread.resume::<()>(());
    assert!(result.is_ok(), "unexpected result: {result:?}");

    Ok(())
}

#[test]
fn test_thread_reset() -> Result<()> {
    use mluau::{AnyUserData, UserData};
    use std::sync::Arc;

    let lua = Lua::new();

    struct MyUserData(#[allow(unused)] Arc<()>);
    impl UserData for MyUserData {}

    let arc = Arc::new(());

    let func: Function = lua.load(r#"function(ud) coroutine.yield(ud) end"#).eval()?;
    let thread = lua.create_thread(lua.load("return 0").into_function()?)?; // Dummy function first
    assert!(thread.reset(func.clone()).is_ok());

    for _ in 0..2 {
        assert_eq!(thread.status(), ThreadStatus::Resumable);
        let _ = thread.resume::<AnyUserData>(MyUserData(arc.clone()))?;
        assert_eq!(thread.status(), ThreadStatus::Resumable);
        assert_eq!(Arc::strong_count(&arc), 2);
        thread.resume::<()>(())?;
        assert_eq!(thread.status(), ThreadStatus::Finished);
        thread.reset(func.clone())?;
        lua.gc_collect()?;
        assert_eq!(Arc::strong_count(&arc), 1);
    }

    // Check for errors
    let func: Function = lua.load(r#"function(ud) error("test error") end"#).eval()?;
    let thread = lua.create_thread(func.clone())?;
    let _ = thread.resume::<AnyUserData>(MyUserData(arc.clone()));
    assert_eq!(thread.status(), ThreadStatus::Error);
    assert_eq!(Arc::strong_count(&arc), 2);
    #[cfg(feature = "lua54")]
    {
        assert!(thread.reset(func.clone()).is_err());
        // Reset behavior has changed in Lua v5.4.4
        // It's became possible to force reset thread by popping error object
        assert!(matches!(thread.status(), ThreadStatus::Finished));
        assert!(thread.reset(func.clone()).is_ok());
        assert_eq!(thread.status(), ThreadStatus::Resumable);
    }
    #[cfg(any(feature = "lua54", feature = "luau"))]
    {
        assert!(thread.reset(func.clone()).is_ok());
        assert_eq!(thread.status(), ThreadStatus::Resumable);
    }

    // Try reset running thread
    let thread = lua.create_thread(lua.create_function(|lua, ()| {
        let this = lua.current_thread();
        this.reset(lua.create_function(|_, ()| Ok(()))?)?;
        Ok(())
    })?)?;
    let result = thread.resume::<()>(());
    assert!(
        matches!(result, Err(Error::CallbackError{ ref cause, ..})
            if matches!(cause.as_ref(), Error::RuntimeError(err)
                if err == "cannot reset a running thread")
        ),
        "unexpected result: {result:?}",
    );

    Ok(())
}

#[test]
fn test_coroutine_from_closure() -> Result<()> {
    let lua = Lua::new();

    let thrd_main = lua.create_function(|_, ()| Ok(()))?;
    lua.globals().set("main", thrd_main)?;

    #[cfg(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "luajit",
        feature = "luau"
    ))]
    let thrd: Thread = lua.load("coroutine.create(main)").eval()?;
    #[cfg(feature = "lua51")]
    let thrd: Thread = lua
        .load("coroutine.create(function(...) return main(unpack(arg)) end)")
        .eval()?;

    thrd.resume::<()>(())?;

    Ok(())
}

#[test]
#[cfg(not(panic = "abort"))]
fn test_coroutine_panic() {
    match catch_unwind(|| -> Result<()> {
        // check that coroutines propagate panics correctly
        let lua = Lua::new();
        let thrd_main = lua.create_function(|_, ()| -> Result<()> {
            panic!("test_panic");
        })?;
        lua.globals().set("main", &thrd_main)?;
        let thrd: Thread = lua.create_thread(thrd_main)?;
        thrd.resume(())
    }) {
        Ok(r) => panic!("coroutine panic not propagated, instead returned {:?}", r),
        Err(p) => assert!(*p.downcast::<&str>().unwrap() == "test_panic"),
    }
}

#[test]
fn test_thread_pointer() -> Result<()> {
    let lua = Lua::new();

    let func = lua.load("return 123").into_function()?;
    let thread = lua.create_thread(func.clone())?;

    assert_eq!(thread.to_pointer(), thread.clone().to_pointer());
    assert_ne!(thread.to_pointer(), lua.current_thread().to_pointer());

    Ok(())
}

#[test]
#[cfg(feature = "luau")]
fn test_thread_resume_error() -> Result<()> {
    let lua = Lua::new();

    let thread = lua
        .load(
            r#"
        coroutine.create(function()
            local ok, err = pcall(coroutine.yield, 123)
            assert(not ok, "yield should fail")
            assert(err == "myerror", "unexpected error: " .. tostring(err))
            return "success"
        end)
    "#,
        )
        .eval::<Thread>()?;

    assert_eq!(thread.resume::<i64>(())?, 123);
    let status = thread.resume_error::<String>("myerror").unwrap();
    assert_eq!(status, "success");

    Ok(())
}

#[test]
fn test_thread_resume_bad_arg() -> Result<()> {
    let lua = Lua::new();

    struct BadArg;

    impl IntoLua for BadArg {
        fn into_lua(self, _lua: &Lua) -> Result<Value> {
            Err(Error::runtime("bad arg"))
        }
    }

    let f = lua.create_thread(lua.create_function(|_, ()| Ok("okay"))?)?;
    let res = f.resume::<()>((123, BadArg));
    assert!(matches!(res, Err(Error::RuntimeError(msg)) if msg == "bad arg"));
    let res = f.resume::<String>(()).unwrap();
    assert_eq!(res, "okay");

    Ok(())
}

#[test]
#[cfg(not(feature = "lua51"))]
fn test_thread_yield_args() -> Result<()> {
    let lua = Lua::new();
    let always_yield = lua.create_function(|lua, ()| lua.yield_with((42, "69420".to_string(), 45.6)))?;

    let thread = lua.create_thread(always_yield)?;
    assert_eq!(
        thread.resume::<(i32, String, f32)>(())?,
        (42, String::from("69420"), 45.6)
    );

    // Assert unlocked
    #[cfg(feature = "send")]
    assert!(
        !lua.is_locked(),
        "Lua state should be unlocked after thread yield"
    );

    // yield, no userdata
    let my_lua_func = lua
        .load(
            r#"
        local my_data = ...
        return my_data()
        "#,
        )
        .into_function()?;

    let thread = lua.create_thread(my_lua_func)?;
    let intermediate =
        thread.resume::<mluau::MultiValue>(lua.create_function(|lua, ()| lua.yield_with(100))?);
    assert!(
        intermediate.is_ok(),
        "Failed to resume thread: {:?}",
        intermediate
    );
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    let result = thread.resume::<i32>(intermediate.unwrap());
    assert!(result.is_ok(), "Failed to resume thread: {:?}", result);
    assert_eq!(result.unwrap(), 100, "Unexpected yield value");
    assert_eq!(thread.status(), ThreadStatus::Finished);

    struct MyTestUserData {
        value: i32,
    }

    impl mluau::UserData for MyTestUserData {
        fn add_methods<M: mluau::UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("yield", |lua, this, ()| {
                println!("yield called");
                lua.yield_with(this.value)?;
                Ok("Thread did not yield")
            });
        }
    }

    let my_data = MyTestUserData { value: 100 };

    let my_lua_func = lua
        .load(
            r#"
        local my_data = ...
        return my_data:yield(12345)
        "#,
        )
        .into_function()?;

    let thread = lua.create_thread(my_lua_func)?;
    let intermediate = thread.resume::<mluau::MultiValue>(my_data);
    println!("intermediate={:?}", intermediate);
    assert!(
        intermediate.is_ok(),
        "Failed to resume thread: {:?}",
        intermediate
    );
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    let result = thread.resume::<i32>(intermediate.unwrap());
    assert!(result.is_ok(), "Failed to resume thread: {:?}", result);
    assert_eq!(result.unwrap(), 100, "Unexpected yield value");
    assert_eq!(thread.status(), ThreadStatus::Finished);

    // Assert unlocked
    #[cfg(feature = "send")]
    assert!(
        !lua.is_locked(),
        "Lua state should be unlocked after thread yield"
    );

    // mlua khvzak yield
    let func = lua.create_function(|lua, ()| lua.yield_with("yielded value"))?;

    let thread = lua.create_thread(func)?;
    assert_eq!(thread.resume::<String>(())?, "yielded value");
    assert_eq!(thread.status(), ThreadStatus::Resumable);
    thread.resume::<()>(())?;
    assert_eq!(thread.status(), ThreadStatus::Finished);

    Ok(())
}

#[test]
#[cfg(all(not(feature = "lua51"), not(feature = "luajit")))]
fn test_continuation() {
    let lua = Lua::new();
    // No yielding continuation fflag test
    let cont_func = lua
        .create_function_with_continuation(
            |lua, a: u64| lua.yield_with(a),
            |_lua, _status, a: u64| {
                println!("Reached cont");
                Ok(a + 39)
            },
        )
        .expect("Failed to create cont_func");

    let luau_func = lua
        .load(
            "
        local cont_func = ...
        local res = cont_func(1)
        return res + 1
    ",
        )
        .into_function()
        .expect("Failed to create function");

    let th = lua
        .create_thread(luau_func)
        .expect("Failed to create luau thread");

    let v = th
        .resume::<mluau::MultiValue>(cont_func)
        .expect("Failed to resume");
    let v = th.resume::<i32>(v).expect("Failed to load continuation");

    assert_eq!(v, 41);

    // empty yield args test
    let cont_func = lua
        .create_function_with_continuation(
            |lua, _: ()| lua.yield_with(()),
            |_lua, _status, mv: mluau::MultiValue| Ok(mv.len()),
        )
        .expect("Failed to create cont_func");

    let luau_func = lua
        .load(
            "
        local cont_func = ...
        local res = cont_func(1)
        return res - 1
    ",
        )
        .into_function()
        .expect("Failed to create function");

    let th = lua
        .create_thread(luau_func)
        .expect("Failed to create luau thread");

    let v = th
        .resume::<mluau::MultiValue>(cont_func)
        .expect("Failed to resume");
    assert!(v.is_empty());
    let v = th.resume::<i32>(v).expect("Failed to load continuation");
    assert_eq!(v, -1);

    let cont_func = lua
        .create_function_with_continuation(
            |_lua, a: u64| Ok(a + 1),
            |_lua, _status, a: u64| {
                println!("Reached cont");
                Ok(a + 2)
            },
        )
        .expect("Failed to create cont_func");

    // Ensure normal calls work still
    assert_eq!(
        lua.load("local cont_func = ...\nreturn cont_func(1)")
            .call::<u64>(cont_func)
            .expect("Failed to call cont_func"),
        2
    );

    // basic yield test before we go any further
    let always_yield = lua
        .create_function(|lua, ()| lua.yield_with((42, "69420".to_string(), 45.6)))
        .unwrap();

    let thread = lua.create_thread(always_yield).unwrap();
    assert_eq!(
        thread.resume::<(i32, String, f32)>(()).unwrap(),
        (42, String::from("69420"), 45.6)
    );

    // Trigger the continuation
    let cont_func = lua
        .create_function_with_continuation(
            |lua, a: u64| lua.yield_with(a),
            |_lua, _status, a: u64| {
                println!("Reached cont");
                Ok(a + 39)
            },
        )
        .expect("Failed to create cont_func");

    let luau_func = lua
        .load(
            "
                local cont_func = ...
                local res = cont_func(1)
                return res + 1
            ",
        )
        .into_function()
        .expect("Failed to create function");

    let th = lua
        .create_thread(luau_func)
        .expect("Failed to create luau thread");

    let v = th
        .resume::<mluau::MultiValue>(cont_func)
        .expect("Failed to resume");
    let v = th.resume::<i32>(v).expect("Failed to load continuation");

    assert_eq!(v, 41);

    let always_yield = lua
        .create_function_with_continuation(
            |lua, ()| lua.yield_with((42, "69420".to_string(), 45.6)),
            |_lua, _, mv: mluau::MultiValue| {
                println!("Reached second continuation");
                if mv.is_empty() {
                    return Ok(mv);
                }
                Err(mluau::Error::external(format!("a{}", mv.len())))
            },
        )
        .unwrap();

    let thread = lua.create_thread(always_yield).unwrap();
    let mv = thread.resume::<mluau::MultiValue>(()).unwrap();
    assert!(thread
        .resume::<String>(mv)
        .unwrap_err()
        .to_string()
        .starts_with("a3"));

    let cont_func = lua
        .create_function_with_continuation(
            |lua, a: u64| lua.yield_with((a + 1, 1)),
            |lua, status, args: mluau::MultiValue| {
                println!("Reached cont recursive/multiple: {:?}", args);

                if args.len() == 5 {
                    if cfg!(any(feature = "luau", feature = "lua52")) {
                        assert_eq!(status, mluau::ContinuationStatus::Ok);
                    } else {
                        assert_eq!(status, mluau::ContinuationStatus::Yielded);
                    }
                    return Ok(6_i32);
                }

                lua.yield_with((args.len() + 1, args))?; // thread state becomes LEN, LEN-1... 1
                Ok(1_i32) // this will be ignored
            },
        )
        .expect("Failed to create cont_func");

    let luau_func = lua
        .load(
            "
                local cont_func = ...
                local res = cont_func(1)
                return res + 1
            ",
        )
        .into_function()
        .expect("Failed to create function");
    let th = lua
        .create_thread(luau_func)
        .expect("Failed to create luau thread");

    let v = th
        .resume::<mluau::MultiValue>(cont_func)
        .expect("Failed to resume");
    println!("v={:?}", v);

    let v = th
        .resume::<mluau::MultiValue>(v)
        .expect("Failed to load continuation");
    println!("v={:?}", v);
    let v = th
        .resume::<mluau::MultiValue>(v)
        .expect("Failed to load continuation");
    println!("v={:?}", v);
    let v = th
        .resume::<mluau::MultiValue>(v)
        .expect("Failed to load continuation");

    // (2, 1) followed by ()
    assert_eq!(v.len(), 2 + 3);

    let v = th.resume::<i32>(v).expect("Failed to load continuation");

    assert_eq!(v, 7);

    // test panics
    let cont_func = lua
        .create_function_with_continuation(
            |lua, a: u64| lua.yield_with(a),
            |_lua, _status, _a: u64| {
                panic!("Reached continuation which should panic!");
                #[allow(unreachable_code)]
                Ok(())
            },
        )
        .expect("Failed to create cont_func");

    let luau_func = lua
        .load(
            "
                local cont_func = ...
                local ok, res = pcall(cont_func, 1)
                assert(not ok)
                return tostring(res)
            ",
        )
        .into_function()
        .expect("Failed to create function");

    let th = lua
        .create_thread(luau_func)
        .expect("Failed to create luau thread");

    let v = th
        .resume::<mluau::MultiValue>(cont_func)
        .expect("Failed to resume");

    let v = th.resume::<String>(v).expect("Failed to load continuation");
    assert!(v.contains("Reached continuation which should panic!"));
}

//#[test]
#[allow(dead_code)] // only enable when wanted, not in CI/default
fn test_large_thread_creation() {
    let lua = Lua::new();
    lua.set_memory_limit(100_000_000_000).unwrap();
    let th1 = lua
        .create_thread(lua.create_function(|_lua, _: ()| Ok(())).unwrap())
        .unwrap();

    let mut this = Vec::new();
    for _i in 1..2000000 {
        let th = lua
            .create_thread(lua.create_function(|_, ()| Ok(())).unwrap())
            .expect("Failed to create thread");
        this.push(th);
    }
    let th2 = lua
        .create_thread(lua.create_function(|_lua, _: ()| Ok(())).unwrap())
        .unwrap();

    for rth in this {
        let dbg_a = format!("{:?}", rth);
        let th_a = format!("{:?}", th1);
        let th_b = format!("{:?}", th2);
        assert!(
            th1 != rth && th2 != rth,
            "Thread {:?} is equal to th1 ({:?}) or th2 ({:?})",
            rth,
            th1,
            th2
        );
        let dbg_b = format!("{:?}", rth);
        let dbg_th1 = format!("{:?}", th1);
        let dbg_th2 = format!("{:?}", th2);

        // Ensure that the PartialEq across auxiliary threads does not affect the values on stack
        // themselves.
        assert_eq!(dbg_a, dbg_b, "Thread {:?} debug format changed", rth);
        assert_eq!(th_a, dbg_th1, "Thread {:?} debug format changed for th1", rth);
        assert_eq!(th_b, dbg_th2, "Thread {:?} debug format changed for th2", rth);
    }

    #[cfg(all(not(feature = "lua51"), not(feature = "luajit")))]
    {
        let cont_func = lua
            .create_function_with_continuation(
                |_lua, a: u64| Ok(a + 1),
                |_lua, _status, a: u64| {
                    println!("Reached cont");
                    Ok(a + 2)
                },
            )
            .expect("Failed to create cont_func");

        // Ensure normal calls work still
        assert_eq!(
            lua.load("local cont_func = ...\nreturn cont_func(1)")
                .call::<u64>(cont_func)
                .expect("Failed to call cont_func"),
            2
        );

        // basic yield test before we go any further
        let always_yield = lua
            .create_function(|lua, ()| lua.yield_with((42, "69420".to_string(), 45.6)))
            .unwrap();

        let thread = lua.create_thread(always_yield).unwrap();
        assert_eq!(
            thread.resume::<(i32, String, f32)>(()).unwrap(),
            (42, String::from("69420"), 45.6)
        );

        // Trigger the continuation
        let cont_func = lua
            .create_function_with_continuation(
                |lua, a: u64| lua.yield_with(a),
                |_lua, _status, a: u64| {
                    println!("Reached cont");
                    Ok(a + 39)
                },
            )
            .expect("Failed to create cont_func");

        let luau_func = lua
            .load(
                "
                    local cont_func = ...
                    local res = cont_func(1)
                    return res + 1
                ",
            )
            .into_function()
            .expect("Failed to create function");

        let th = lua
            .create_thread(luau_func)
            .expect("Failed to create luau thread");

        let v = th
            .resume::<mluau::MultiValue>(cont_func)
            .expect("Failed to resume");
        let v = th.resume::<i32>(v).expect("Failed to load continuation");

        assert_eq!(v, 41);

        let always_yield = lua
            .create_function_with_continuation(
                |lua, ()| lua.yield_with((42, "69420".to_string(), 45.6)),
                |_lua, _, mv: mluau::MultiValue| {
                    println!("Reached second continuation");
                    if mv.is_empty() {
                        return Ok(mv);
                    }
                    Err(mluau::Error::external(format!("a{}", mv.len())))
                },
            )
            .unwrap();

        let thread = lua.create_thread(always_yield).unwrap();
        let mv = thread.resume::<mluau::MultiValue>(()).unwrap();
        assert!(thread
            .resume::<String>(mv)
            .unwrap_err()
            .to_string()
            .starts_with("a3"));

        let cont_func = lua
            .create_function_with_continuation(
                |lua, a: u64| lua.yield_with((a + 1, 1)),
                |lua, status, args: mluau::MultiValue| {
                    println!("Reached cont recursive/multiple: {:?}", args);

                    if args.len() == 5 {
                        if cfg!(any(feature = "luau", feature = "lua52")) {
                            assert_eq!(status, mluau::ContinuationStatus::Ok);
                        } else {
                            assert_eq!(status, mluau::ContinuationStatus::Yielded);
                        }
                        return Ok(6_i32);
                    }

                    lua.yield_with((args.len() + 1, args))?; // thread state becomes LEN, LEN-1... 1
                    Ok(1_i32) // this will be ignored
                },
            )
            .expect("Failed to create cont_func");

        let luau_func = lua
            .load(
                "
                    local cont_func = ...
                    local res = cont_func(1)
                    return res + 1
                ",
            )
            .into_function()
            .expect("Failed to create function");
        let th = lua
            .create_thread(luau_func)
            .expect("Failed to create luau thread");

        let v = th
            .resume::<mluau::MultiValue>(cont_func)
            .expect("Failed to resume");
        println!("v={:?}", v);

        let v = th
            .resume::<mluau::MultiValue>(v)
            .expect("Failed to load continuation");
        println!("v={:?}", v);
        let v = th
            .resume::<mluau::MultiValue>(v)
            .expect("Failed to load continuation");
        println!("v={:?}", v);
        let v = th
            .resume::<mluau::MultiValue>(v)
            .expect("Failed to load continuation");

        // (2, 1) followed by ()
        assert_eq!(v.len(), 2 + 3);

        let v = th.resume::<i32>(v).expect("Failed to load continuation");

        assert_eq!(v, 7);

        // test panics
        let cont_func = lua
            .create_function_with_continuation(
                |lua, a: u64| lua.yield_with(a),
                |_lua, _status, _a: u64| {
                    panic!("Reached continuation which should panic!");
                    #[allow(unreachable_code)]
                    Ok(())
                },
            )
            .expect("Failed to create cont_func");

        let luau_func = lua
            .load(
                "
                    local cont_func = ...
                    local ok, res = pcall(cont_func, 1)
                    assert(not ok)
                    return tostring(res)
                ",
            )
            .into_function()
            .expect("Failed to create function");

        let th = lua
            .create_thread(luau_func)
            .expect("Failed to create luau thread");

        let v = th
            .resume::<mluau::MultiValue>(cont_func)
            .expect("Failed to resume");

        let v = th.resume::<String>(v).expect("Failed to load continuation");
        assert!(v.contains("Reached continuation which should panic!"));
    }
}

#[test]
#[cfg(feature = "luau")]
pub fn test_thread_set_thread_data() -> Result<()> {
    use std::sync::{Arc, atomic::AtomicU64};

    let lua = Lua::new();

    let thread = lua.create_thread(lua.load(r#""#).into_function()?)?;

    let count = Arc::new(AtomicU64::new(0));
    pub struct TestData {
        pub value: Arc<AtomicU64>,
    }

    impl Drop for TestData {
        fn drop(&mut self) {
            self.value.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    thread.set_thread_data(TestData {
        value: count.clone(),
    })?;

    // Check if we can get a ref to TestData
    {
        use mluau::XRc;

        let data = thread.thread_data::<TestData>().ok_or(Error::runtime("No thread data found"))?;
        assert!(Arc::ptr_eq(&data.value, &count));
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);

        // Now ensure take works with the ref from above also being alive
        let take_data = thread.take_thread_data::<TestData>().ok_or(Error::runtime("No thread data found"))?;
        assert!(Arc::ptr_eq(&take_data.value, &count));
        assert!(Arc::ptr_eq(&data.value, &count));
        assert_eq!(data.value.load(std::sync::atomic::Ordering::SeqCst), 0);
        drop(data); // Drop the ref we got earlier

        // Put it back
        let take_data = XRc::into_inner(take_data).ok_or(Error::runtime("Failed to get inner TestData"))?;
        thread.set_thread_data(take_data)?;
    }

    drop(thread); // This should lead to thread being collected 

    lua.gc_collect()?;
    lua.gc_collect()?;
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);

    let thread_2 = lua.create_thread(lua.load(r#""#).into_function()?)?;
    let count_2 = Arc::new(AtomicU64::new(0));
    thread_2.set_thread_data(TestData {
        value: count_2.clone(),
    })?;

    {
        let data = thread_2.take_thread_data::<TestData>().ok_or(Error::runtime("No thread data found"))?;
        assert!(Arc::ptr_eq(&data.value, &count_2));
        assert_eq!(count_2.load(std::sync::atomic::Ordering::SeqCst), 0);
        thread_2.set_thread_data(data)?; // Put it back
    }

    drop(lua); // This should also lead to thread being collected

    assert_eq!(count_2.load(std::sync::atomic::Ordering::SeqCst), 1);

    Ok(())
}