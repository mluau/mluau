# mluau

<!-- [![Build Status]][github-actions] [![Latest Version]][crates.io] [![API Documentation]][docs.rs] [![Coverage Status]][codecov.io] ![MSRV] -->

[![Build Status]][github-actions] [![API Documentation]][docs.rs] ![MSRV]

[Build Status]: https://github.com/mluau/mluau/workflows/CI/badge.svg 
[github-actions]: https://github.com/mluau/mluau/actions 

<!-- [Latest Version]: https://img.shields.io/crates/v/mlua.svg
[crates.io]: https://crates.io/crates/mlua -->

[API Documentation]: https://docs.rs/mlua/badge.svg
[docs.rs]: https://docs.rs/mlua

<!-- [Coverage Status]: https://codecov.io/gh/mluau/mluau/branch/main/graph/badge.svg?token=99339FS1CG
[codecov.io]: https://codecov.io/gh/mluau/mlua -->

[MSRV]: https://img.shields.io/badge/rust-1.79+-brightgreen.svg?&logo=rust

[Guided Tour] | [Benchmarks] | [FAQ]

[Guided Tour]: examples/guided_tour.rs
[Benchmarks]: https://github.com/khvzak/script-bench-rs
[FAQ]: FAQ.md

This repository is a fork of `mlua` with sole focus on Luau, with the following changes (so far):

- More reliable coroutine and yielding support:
  - `mluau` allows Rust functions to yield back to Luau directly, improving support for iterators, coroutines, and task schedulers.
  - Support for Luau continuations - a Luau feature that allows a yielded Luau thread to call a Rust continuation function upon `coroutine.resume`, before resuming back to Luau.
- Thread stack optimizations and bug fixes:
  - Removes unnecessary copies of the main thread stack to improve resume/yield performance.
  - Uses an auxiliary thread list to prevent panicking if user code makes more than 1 million references to Rust-side code.
- _Removal of async support._
  - `mlua`'s async implementation is prone to freezes and deadlocks, and doesn't fit in as well as we'd like with Luau and the Luau ecosystem in mind.
  - Not to worry! We're looking to replace it with a dedicated Luau-focused scheduler in the future, and are working on making sure it's rock solid just like the rest of Luau.
- Improved adherence to Luau spec to minimize UB and allow for a more easily sandboxed Luau environment:
  - Removal of the `__gc` metamethod on userdata; although implemented by mlua, [should not be supported in Luau](https://luau.org/sandbox#__gc) due to memory safety and optimization considerations.
  - `collectgarbage` now limited to options `"count"` and `"collect"` for better sandboxing. Importantly, this disallows user code from purposely stopping the garbage collector, even when sandbox mode is disabled.
- Removal of `Lua::scope`, a feature we don't use that carried a slight performance penalty.
- Support for getting metatable of non-mlua/non-Rust userdata via the unsafe `AnyUserData::underlying_metatable` method. This is useful for managing `newproxy` userdata's etc.
- `Thread::pop_results` has been added to allow popping results directly from the thread's stack to a `R` which implements `FromLua`. This should not be needed much outside this in practice.
- [`Thread::close`](https://github.com/mlua-rs/mlua/pull/517) has been added to allow closing Lua threads
- `RawLua::stack_value` correctly calls `lua_checkstack` to avoid a potential crash when there are no stack slots free when popping from the Lua stack (`from_lua` etc.)
- Namecall optimization on Luau: for methods/functions on userdata, the `namecall` metamethod is now defined. This allows for more efficient method calls on userdata, as it avoids the need to check for the `__index` metamethod on every call. This is particularly useful for performance-critical code that relies heavily on userdata methods. This optimization is enabled by default, but can be disabled using `UserDataRegistry::disable_namecall_optimization()` if needed.
- Due to namcall, `RawUserDataRegistry` is not `Send`.
- Support for disabling use of a ``Error`` userdata in favor of just stringifying the error. This is useful as ``Error`` userdata tends to have issues with ``xpcall`` depending on the error function handler being used.
- Support for creating tracebacks on the current thread using ``Lua::traceback`` and ``Thread::traceback``.
- ``strong_count`` and ``weak_count`` have been added to both ``WeakLua`` and ``Lua`` for debugging purposes/allowing debugging of Lua VM reference leaks etc.
- Support for getting userdata type name via ``AnyUserData::type_name``
- Support for dynamic userdata. A dynamic userdata is a userdata whose internals are not known at compile time and can hence store arbitrary data and fields. A dynamic userdata is created using the `Lua::create_dynamic_userdata` method, which takes the associated data to store for the userdata and a metatable. The metatable can be used to define methods and fields for the dynamic userdata. The associated data can be accessed using the `AnyUserData::dynamic_data` method, which returns a reference to the associated data to hence allow for functions that 
operate on the dynamic userdata.
- Support for getting weak_lua from Threads and other primitives
- Support for GC interrupts in Luau.
- Sole focus on Luau (non-luau code removed)
- Support for a proper `none` type primitive (`Value::None`), distinguishing it from `nil` or lack of value (custom to our fork of Luau).
- Support for externally managed buffers, enabling zero-copy sharing of memory buffers from Rust to Luau with customizable deallocation callbacks.

As an example of dynamic userdata:

```rust
    let mt1 = lua.create_table()?;
    mt1.set("__type", "my_dynamic_userdata2")?;

    let index_tab = lua.create_table()?;
    index_tab.set("foo", 123)?;
    index_tab.set("bar", lua.create_function(|_lua, ud: AnyUserData| {
        let dt = ud.dynamic_data::<MyDynamicData>()?;
        Ok(dt.foo)
    })?)?;
    mt1.set("__index", index_tab)?;

    let dynamic_userdata = lua.create_dynamic_userdata(MyDynamicData { foo: 124 }, &mt1)?;

    let func = lua.load("local ud = ...; return ud.foo").into_function()?;
    assert_eq!(func.call::<i64>(dynamic_userdata.clone())?, 123);

    let func = lua.load("local ud = ...; return ud:bar()").into_function()?;
    assert_eq!(func.call::<i64>(dynamic_userdata.clone())?, 124);
```

## Roadmap

- Dedicated scheduler for `mluau`
- Tagged userdata (performance optimization)

## The below is `mlua`'s last README which should still be accurate or mostly accurate to `mluau`

> **Note**
>
> See v0.10 [release notes](https://github.com/mlua/mlua/blob/main/docs/release_notes/v0.10.md).

`mlua` is a set of bindings to the [Lua](https://www.lua.org) programming language for Rust with a goal of providing a
_safe_ (as much as possible), high level, easy to use, practical and flexible API.

Started as an `rlua` fork, `mlua` supports Lua 5.4, 5.3, 5.2, 5.1 (including LuaJIT) and [Luau] and allows writing native Lua modules in Rust as well as using Lua in a standalone mode.

`mlua` is tested on Windows/macOS/Linux including module mode in [GitHub Actions] on `x86_64` platforms and cross-compilation to `aarch64` (other targets are also supported).

WebAssembly (WASM) is supported through the `wasm32-unknown-emscripten` target for all Lua/Luau versions excluding JIT.

[GitHub Actions]: https://github.com/mluau/mlua/actions
[Luau]: https://luau.org

## Usage

### Feature flags

`mlua` uses feature flags to reduce the number of dependencies and compiled code, and allow choosing only the required set of features.
Below is a list of the available feature flags. By default `mlua` does not enable any features.

- `luau`: enable [Luau] support (auto vendored mode)
- `luau-jit`: enable [Luau] support with JIT backend.
- `luau-vector4`: enable [Luau] support with 4-dimensional vector.
<!-- * `async`: enable async/await support (any executor can be used, eg. [tokio] or [async-std]) -->
- `send`: make `mluau::Lua: Send + Sync` (adds [`Send`] requirement to `mluau::Function` and `mluau::UserData`)
- `error-send`: make `mlua:Error: Send + Sync`
- `serde`: add serialization and deserialization support to `mlua` types using [serde]
- `macros`: enable procedural macros (such as `chunk!`)
- `anyhow`: enable `anyhow::Error` conversion into Lua
- `userdata-wrappers`: opt into `impl UserData` for `Rc<T>`/`Arc<T>`/`Rc<RefCell<T>>`/`Arc<Mutex<T>>` where `T: UserData`

[`Send`]: https://doc.rust-lang.org/std/marker/trait.Send.html
[serde]: https://github.com/serde-rs/serde

### Serialization (serde) support

With the `serde` feature flag enabled, `mlua` allows you to serialize/deserialize any type that implements [`serde::Serialize`] and [`serde::Deserialize`] into/from [`mluau::Value`]. In addition, `mlua` provides the [`serde::Serialize`] trait implementation for it (including `UserData` support).

[Example](examples/serde.rs)

[`serde::Serialize`]: https://docs.serde.rs/serde/ser/trait.Serialize.html
[`serde::Deserialize`]: https://docs.serde.rs/serde/de/trait.Deserialize.html
[`mluau::Value`]: https://docs.rs/mlua/latest/mlua/enum.Value.html

## Safety

One of `mlua`'s goals is to provide a _safe_ API between Rust and Lua.
Every place where the Lua C API may trigger an error longjmp is protected by `lua_pcall`,
and the user of the library is protected from directly interacting with unsafe things like the Lua stack.
There is overhead associated with this safety.

Unfortunately, `mlua` does not provide absolute safety even without using `unsafe` .
This library contains a huge amount of unsafe code. There are almost certainly bugs still lurking in this library!
It is surprisingly, fiendishly difficult to use the Lua C API without the potential for unsafety.

## Panic handling

`mlua` wraps panics that are generated inside Rust callbacks in a regular Lua error. Panics can then be
resumed by returning or propagating the Lua error to Rust code.

For example:

```rust
let lua = Lua::new();
let f = lua.create_function(|_, ()| -> LuaResult<()> {
    panic!("test panic");
})?;
lua.globals().set("rust_func", f)?;

let _ = lua.load(r#"
    local status, err = pcall(rust_func)
    print(err) -- prints: test panic
    error(err) -- propagate panic
"#).exec();

unreachable!()
```

Optionally, `mlua` can disable Rust panic catching in Lua via `pcall`/`xpcall` and automatically resume
them across the Lua API boundary. This is controlled via `LuaOptions` and done by wrapping the Lua `pcall`/`xpcall`
functions to prevent catching errors that are wrapped Rust panics.

`mlua` should also be panic safe in another way as well, which is that any `Lua` instances or handles
remain usable after a user generated panic, and such panics should not break internal invariants or
leak Lua stack space. This is mostly important to safely use `mlua` types in Drop impls, as you should not be
using panics for general error handling.

Below is a list of `mlua` behaviors that should be considered bugs.
If you encounter them, a bug report would be very welcome:

- If you can cause UB with `mlua` without typing the word "unsafe", this is a bug.

- If your program panics with a message that contains the string "mlua internal error", this is a bug.

- Lua C API errors are handled by longjmp. All instances where the Lua C API would otherwise longjmp over calling stack frames should be guarded against, except in internal callbacks where this is intentional. If you detect that `mlua` is triggering a longjmp over your Rust stack frames, this is a bug!

- If you detect that, after catching a panic or during a Drop triggered from a panic, a `Lua` or handle method is triggering other bugs or there is a Lua stack space leak, this is a bug. `mlua` instances are supposed to remain fully usable in the face of user generated panics. This guarantee does not extend to panics marked with "mlua internal error" simply because that is already indicative of a separate bug.

## Sandboxing

Please check the [Luau Sandboxing] page if you are interested in running untrusted Lua scripts in a controlled environment.

`mlua` provides the `Lua::sandbox` method for enabling sandbox mode (Luau only).

[Luau Sandboxing]: https://luau.org/sandbox

## License

This project is licensed under the [MIT license](LICENSE).
