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

This repository is a fork of `mlua` with a greater focus on Luau, with the following changes (so far):

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
- Integration with the [Lute](https://github.com/luau-lang/lute) runtime and scheduler via the `luau-lute` feature flag. Note that crypto and net are disabled by default due to increasing compiler times and leading to large memory usage during linking, if you want to enable crypto and net, set the `luau-lute-crypto` and `luau-lute-net` flags respectively. Prebuilt static libraries of Lute are available for Linux (GNU, x86_64 and aarch64) and Windows (x86_64) via ``luau-lute-prebuilt`` feature flag. Note that both Linux and Windows prebuilt libraries are highly experimental and may not work as expected, please report any issues you encounter.
- Support for getting metatable of non-mlua/non-Rust userdata via the unsafe `AnyUserData::underlying_metatable` method. This is useful for managing `newproxy` and (Luau only) Lute userdata.
- `Thread::pop_results` has been added to allow popping results directly from the thread's stack to a `R` which implements `FromLua`. This is useful when trying to interoperate with Lute runtime but should not be needed much outside this in practice.
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
- Integration with C++ tooling, most importantly Lute, the Luau language's official general purpose runtime for Luau.
  - Support for Luau AST, Compiler, etc. reflection through Lute.
- Tagged userdata (performance optimization)

## The below is `mlua`'s last README which should still be accurate or mostly accurate to `mluau`

> **Note**
>
> See v0.10 [release notes](https://github.com/mlua/mlua/blob/main/docs/release_notes/v0.10.md).

`mlua` is a set of bindings to the [Lua](https://www.lua.org) programming language for Rust with a goal to provide a
_safe_ (as much as possible), high level, easy to use, practical and flexible API.

Started as an `rlua` fork, `mlua` supports Lua 5.4, 5.3, 5.2, 5.1 (including LuaJIT) and [Luau] and allows writing native Lua modules in Rust as well as using Lua in a standalone mode.

`mlua` is tested on Windows/macOS/Linux including module mode in [GitHub Actions] on `x86_64` platforms and cross-compilation to `aarch64` (other targets are also supported).

WebAssembly (WASM) is supported through `wasm32-unknown-emscripten` target for all Lua/Luau versions excluding JIT.

[GitHub Actions]: https://github.com/mluau/mlua/actions
[Luau]: https://luau.org

## Usage

### Feature flags

`mlua` uses feature flags to reduce the amount of dependencies and compiled code, and allow to choose only required set of features.
Below is a list of the available feature flags. By default `mlua` does not enable any features.

- `lua54`: enable Lua [5.4] support
- `lua53`: enable Lua [5.3] support
- `lua52`: enable Lua [5.2] support
- `lua51`: enable Lua [5.1] support
- `luajit`: enable [LuaJIT] support
- `luajit52`: enable [LuaJIT] support with partial compatibility with Lua 5.2
- `luau`: enable [Luau] support (auto vendored mode)
- `luau-jit`: enable [Luau] support with JIT backend.
- `luau-vector4`: enable [Luau] support with 4-dimensional vector.
- `vendored`: build static Lua(JIT) libraries from sources during `mlua` compilation using [lua-src] or [luajit-src]
- `module`: enable module mode (building loadable `cdylib` library for Lua)
<!-- * `async`: enable async/await support (any executor can be used, eg. [tokio] or [async-std]) -->
- `send`: make `mluau::Lua: Send + Sync` (adds [`Send`] requirement to `mluau::Function` and `mluau::UserData`)
- `error-send`: make `mlua:Error: Send + Sync`
- `serde`: add serialization and deserialization support to `mlua` types using [serde]
- `macros`: enable procedural macros (such as `chunk!`)
- `anyhow`: enable `anyhow::Error` conversion into Lua
- `userdata-wrappers`: opt into `impl UserData` for `Rc<T>`/`Arc<T>`/`Rc<RefCell<T>>`/`Arc<Mutex<T>>` where `T: UserData`

[5.4]: https://www.lua.org/manual/5.4/manual.html
[5.3]: https://www.lua.org/manual/5.3/manual.html
[5.2]: https://www.lua.org/manual/5.2/manual.html
[5.1]: https://www.lua.org/manual/5.1/manual.html
[LuaJIT]: https://luajit.org/
[lua-src]: https://github.com/mlua-rs/lua-src-rs
[luajit-src]: https://github.com/mlua-rs/luajit-src-rs
[`Send`]: https://doc.rust-lang.org/std/marker/trait.Send.html
[serde]: https://github.com/serde-rs/serde

### Serialization (serde) support

With the `serde` feature flag enabled, `mlua` allows you to serialize/deserialize any type that implements [`serde::Serialize`] and [`serde::Deserialize`] into/from [`mluau::Value`]. In addition, `mlua` provides the [`serde::Serialize`] trait implementation for it (including `UserData` support).

[Example](examples/serde.rs)

[`serde::Serialize`]: https://docs.serde.rs/serde/ser/trait.Serialize.html
[`serde::Deserialize`]: https://docs.serde.rs/serde/de/trait.Deserialize.html
[`mluau::Value`]: https://docs.rs/mlua/latest/mlua/enum.Value.html

### Compiling

You have to enable one of the features: `lua54`, `lua53`, `lua52`, `lua51`, `luajit(52)` or `luau`, according to the chosen Lua version.

By default `mlua` uses `pkg-config` to find Lua includes and libraries for the chosen Lua version.
In most cases it works as desired, although sometimes it may be preferable to use a custom Lua library.
To achieve this, mlua supports the `LUA_LIB`, `LUA_LIB_NAME` and `LUA_LINK` environment variables.
`LUA_LINK` is optional and may be `dylib` (a dynamic library) or `static` (a static library, `.a` archive).

An example of how to use them:

```sh
my_project $ LUA_LIB=$HOME/tmp/lua-5.2.4/src LUA_LIB_NAME=lua LUA_LINK=static cargo build
```

`mlua` also supports vendored Lua/LuaJIT using the auxiliary crates [lua-src](https://crates.io/crates/lua-src) and
[luajit-src](https://crates.io/crates/luajit-src).
Just enable the `vendored` feature and cargo will automatically build and link the specified Lua/LuaJIT version. This is the easiest way to get started with `mlua`.

### Standalone mode

In standalone mode, `mlua` allows adding scripting support to your application with a gently configured Lua runtime to ensure safety and soundness.

Add to `Cargo.toml`:

```toml
[dependencies]
mlua = { version = "0.11", features = ["lua54", "vendored"] }
```

`main.rs`

```rust
use mluau::prelude::*;

fn main() -> LuaResult<()> {
    let lua = Lua::new();

    let map_table = lua.create_table()?;
    map_table.set(1, "one")?;
    map_table.set("two", 2)?;

    lua.globals().set("map_table", map_table)?;

    lua.load("for k,v in pairs(map_table) do print(k,v) end").exec()?;

    Ok(())
}
```

### Module mode

In module mode, `mlua` allows creating a compiled Lua module that can be loaded from Lua code using [`require`](https://www.lua.org/manual/5.4/manual.html#pdf-require). In this case `mlua` uses an external Lua runtime which could lead to potential unsafety due to the unpredictability of the Lua environment and usage of libraries such as [`debug`](https://www.lua.org/manual/5.4/manual.html#6.10).

[Example](examples/module)

Add to `Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
mlua = { version = "0.11", features = ["lua54", "module"] }
```

`lib.rs`:

```rust
use mluau::prelude::*;

fn hello(_: &Lua, name: String) -> LuaResult<()> {
    println!("hello, {}!", name);
    Ok(())
}

#[mluau::lua_module]
fn my_module(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set("hello", lua.create_function(hello)?)?;
    Ok(exports)
}
```

And then (**macOS** example):

```sh
$ cargo rustc -- -C link-arg=-undefined -C link-arg=dynamic_lookup
$ ln -s ./target/debug/libmy_module.dylib ./my_module.so
$ lua5.4 -e 'require("my_module").hello("world")'
hello, world!
```

On macOS, you need to set additional linker arguments. One option is to compile with `cargo rustc --release -- -C link-arg=-undefined -C link-arg=dynamic_lookup`, the other is to create a `.cargo/config.toml` with the following content:

```toml
[target.x86_64-apple-darwin]
rustflags = [
  "-C", "link-arg=-undefined",
  "-C", "link-arg=dynamic_lookup",
]

[target.aarch64-apple-darwin]
rustflags = [
  "-C", "link-arg=-undefined",
  "-C", "link-arg=dynamic_lookup",
]
```

On Linux you can build modules normally with `cargo build --release`.

On Windows the target module will be linked with the `lua5x.dll` library (depending on your feature flags).
Your main application should provide this library.

Module builds don't require Lua binaries or headers to be installed on the system.

### Publishing to luarocks.org

There is a LuaRocks build backend for mlua modules: [`luarocks-build-rust-mlua`].

Modules written in Rust and published to luarocks:

- [`decasify`](https://github.com/alerque/decasify)
- [`lua-ryaml`](https://github.com/khvzak/lua-ryaml)
- [`tiktoken_core`](https://github.com/gptlang/lua-tiktoken)
- [`toml-edit`](https://github.com/vhyrro/toml-edit.lua)
- [`typst-lua`](https://github.com/rousbound/typst-lua)

[`luarocks-build-rust-mlua`]: https://luarocks.org/modules/khvzak/luarocks-build-rust-mlua

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

Below is a list of `mlua` behaviors that should be considered a bug.
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
