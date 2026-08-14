#![cfg(feature = "send")]

use std::string::String as StdString;

use mluau::{AnyUserData, Error, Lua, Result, UserData, UserDataMethods, UserDataRef, Function};
use static_assertions::{assert_impl_all, assert_not_impl_all};

#[test]
fn test_userdata_multithread_access_sync() -> Result<()> {
    let lua = Lua::new();

    // This type is `Send` and `Sync`.
    struct MyUserData(StdString);
    assert_impl_all!(MyUserData: Send, Sync);

    impl UserData for MyUserData {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            methods.add_method("method", |lua, this, ()| {
                let ud = lua.globals().get::<AnyUserData>("ud")?;
                assert!(ud.get::<Function>("method2")?.call::<()>((ud)).is_ok());
                Ok(this.0.clone())
            });

            methods.add_method("method2", |_, _, ()| Ok(()));
        }
    }

    lua.globals().set("ud", MyUserData("hello".to_string()))?;

    // We acquired the shared reference.
    let _ud = lua.globals().get::<UserDataRef<MyUserData>>("ud")?;

    std::thread::scope(|s| {
        s.spawn(|| {
            // Getting another shared reference for `Sync` type is allowed.
            let _ = lua.globals().get::<UserDataRef<MyUserData>>("ud").unwrap();
        });
    });

    lua.load("ud:method()").exec().unwrap();

    Ok(())
}
