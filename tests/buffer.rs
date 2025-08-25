#![cfg(feature = "luau")]

use mluau::{Lua, Result, Value};
use std::io::{Read, Seek, SeekFrom, Write};

#[test]
fn test_buffer() -> Result<()> {
    let lua = Lua::new();

    let buf1 = lua
        .load(
            r#"
        local buf = buffer.fromstring("hello")
        assert(buffer.len(buf) == 5)
        return buf
    "#,
        )
        .eval::<Value>()?;
    assert!(buf1.is_buffer());
    assert_eq!(buf1.type_name(), "buffer");

    let buf2 = lua.load("buffer.fromstring('hello')").eval::<Value>()?;
    assert_ne!(buf1, buf2);

    // Check that we can pass buffer type to Lua
    let buf1 = buf1.as_buffer().unwrap();
    let func = lua.create_function(|_, buf: Value| return buf.to_string())?;
    assert!(func.call::<String>(buf1)?.starts_with("buffer:"));

    // Check buffer methods
    assert_eq!(buf1.len(), 5);
    assert_eq!(buf1.to_vec(), b"hello");
    assert_eq!(buf1.read_bytes::<3>(1), [b'e', b'l', b'l']);
    assert_eq!(buf1.read_bytes_to_vec(1, 3), b"ell".to_vec());
    buf1.write_bytes(1, b"i");
    assert_eq!(buf1.to_vec(), b"hillo");

    let buf3 = lua.create_buffer(b"")?;
    assert!(buf3.is_empty());

    let p = buf3.to_pointer();
    assert!(!p.is_null());

    assert!(!Value::Buffer(buf3).to_pointer().is_null());

    Ok(())
}

#[test]
#[should_panic]
fn test_buffer_out_of_bounds_read() {
    let lua = Lua::new();
    let buf = lua.create_buffer(b"hello, world!").unwrap();
    _ = buf.read_bytes::<1>(13);
}

#[test]
#[should_panic]
fn test_buffer_out_of_bounds_write() {
    let lua = Lua::new();
    let buf = lua.create_buffer(b"hello, world!").unwrap();
    buf.write_bytes(14, b"!!");
}

#[test]
fn create_large_buffer() {
    let lua = Lua::new();
    let err = lua.create_buffer_with_capacity(1_073_741_824 + 1).unwrap_err(); // 1GB
    assert!(err.to_string().contains("memory allocation error"));

    // Normal buffer is okay
    let buf = lua.create_buffer_with_capacity(1024 * 1024).unwrap();
    assert_eq!(buf.len(), 1024 * 1024);
}

#[test]
fn test_buffer_cursor() -> Result<()> {
    let lua = Lua::new();
    let mut cursor = lua.create_buffer(b"hello, world")?.cursor();

    let mut data = Vec::new();
    cursor.read_to_end(&mut data)?;
    assert_eq!(data, b"hello, world");

    // No more data to read
    let mut one = [0u8; 1];
    assert_eq!(cursor.read(&mut one)?, 0);

    // Seek to start
    cursor.seek(SeekFrom::Start(0))?;
    cursor.read_exact(&mut one)?;
    assert_eq!(one, [b'h']);

    // Seek to end -5
    cursor.seek(SeekFrom::End(-5))?;
    let mut five = [0u8; 5];
    cursor.read_exact(&mut five)?;
    assert_eq!(&five, b"world");

    // Seek to current -1
    cursor.seek(SeekFrom::Current(-1))?;
    cursor.read_exact(&mut one)?;
    assert_eq!(one, [b'd']);

    // Invalid seek
    assert!(cursor.seek(SeekFrom::Current(-100)).is_err());
    assert!(cursor.seek(SeekFrom::End(1)).is_err());

    // Write data
    let buf = lua.create_buffer_with_capacity(100)?;
    cursor = buf.clone().cursor();

    cursor.write_all(b"hello, ...")?;
    cursor.seek(SeekFrom::Current(-3))?;
    cursor.write_all(b"Rust!")?;

    assert_eq!(&buf.read_bytes::<12>(0), b"hello, Rust!");

    // Writing beyond the end of the buffer does nothing
    cursor.seek(SeekFrom::End(0))?;
    assert_eq!(cursor.write(b".")?, 0);

    // Flush is no-op
    cursor.flush()?;

    Ok(())
}
