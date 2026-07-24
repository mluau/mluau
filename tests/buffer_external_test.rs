use mluau::prelude::*;

#[test]
fn test_external_buffer_tostring() {
    let lua = Lua::new();
    let b = bytes::Bytes::from("hello world");
    let buf = lua.create_external_buffer(b).unwrap();
    lua.globals().set("buf", buf).unwrap();
    let res: String = lua.load("return buffer.tostring(buf)").eval().unwrap();
    assert_eq!(res, "hello world");

    let len: usize = lua.load("return buffer.len(buf)").eval().unwrap();
    assert_eq!(len, 11);
}
