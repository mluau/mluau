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

#[test]
fn test_external_buffer() -> Result<()> {
    let lua = Lua::new();
    let data = b"hello, world".to_vec();
    let buf = lua.create_external_buffer(data)?;

    assert_eq!(buf.len(), 12);
    assert_eq!(buf.to_vec(), b"hello, world");

    // Ensure immutable buffers are in fact immutable
    let err = lua.load("local b = ...; buffer.writeu8(b, 0, 42)")
        .call::<()>(buf.clone())
        .unwrap_err();
    assert!(err.to_string().contains("immutable"));

    // Check reading in a loop from Luau
    let sum: u32 = lua.load(r#"
        local b = ...
        local sum = 0
        for i=0, buffer.len(b)-1 do
            sum = sum + buffer.readu8(b, i)
        end
        return sum
    "#).call(buf.clone())?;

    let expected_sum = b"hello, world".iter().map(|&b| b as u32).sum::<u32>();
    assert_eq!(sum, expected_sum);

    // Ensure memory lifecycle works on GC
    drop(buf);
    lua.gc_collect()?;

    Ok(())
}

#[test]
fn test_external_buffer_mut() -> Result<()> {
    let lua = Lua::new();
    let data = b"hello, world".to_vec();
    let buf = lua.create_external_buffer_mut(data)?;

    assert_eq!(buf.len(), 12);
    assert_eq!(buf.to_vec(), b"hello, world");

    // Check mutability
    lua.load("local b = ...; buffer.writeu8(b, 0, 72)")
        .call::<()>(buf.clone())?;

    assert_eq!(buf.read_bytes::<1>(0), [72]);
    assert_eq!(&buf.to_vec()[..5], b"Hello");

    // Check reading in a loop from Luau
    let sum: u32 = lua.load(r#"
        local b = ...
        local sum = 0
        for i=0, buffer.len(b)-1 do
            sum = sum + buffer.readu8(b, i)
        end
        return sum
    "#).call(buf.clone())?;

    let expected_sum = b"Hello, world".iter().map(|&b| b as u32).sum::<u32>();
    assert_eq!(sum, expected_sum);

    // Ensure memory lifecycle works on GC
    drop(buf);
    lua.gc_collect()?;

    Ok(())
}

#[cfg(feature = "bytes")]
#[test]
fn test_external_buffer_bytes() -> Result<()> {
    let lua = Lua::new();
    let data = bytes::Bytes::from_static(b"hello, world");
    let buf = lua.create_external_buffer(data)?;

    assert_eq!(buf.len(), 12);
    assert_eq!(buf.to_vec(), b"hello, world");

    // Check reading in a loop from Luau
    let sum: u32 = lua.load(r#"
        local b = ...
        local sum = 0
        for i=0, buffer.len(b)-1 do
            sum = sum + buffer.readu8(b, i)
        end
        return sum
    "#).call(buf.clone())?;

    let expected_sum = b"hello, world".iter().map(|&b| b as u32).sum::<u32>();
    assert_eq!(sum, expected_sum);

    // GC
    drop(buf);
    lua.gc_collect()?;

    Ok(())
}

#[cfg(feature = "bytes")]
#[test]
fn test_external_buffer_bytes_sliced_and_cloned() -> Result<()> {
    let lua = Lua::new();
    let original_data = bytes::Bytes::from_static(b"prefix: hello, world :suffix");
    
    // Slice it to get a shifted pointer
    let sliced_data = original_data.slice(8..20);
    
    // Clone it
    let cloned_data = sliced_data.clone();

    // Use sliced_data in one buffer
    let buf1 = lua.create_external_buffer(sliced_data)?;
    assert_eq!(buf1.len(), 12);
    assert_eq!(buf1.to_vec(), b"hello, world");

    // Use cloned_data in another buffer
    let buf2 = lua.create_external_buffer(cloned_data)?;
    assert_eq!(buf2.len(), 12);
    assert_eq!(buf2.to_vec(), b"hello, world");

    // Check reading in Luau
    let sum1: u32 = lua.load(r#"
        local b = ...
        local sum = 0
        for i=0, buffer.len(b)-1 do
            sum = sum + buffer.readu8(b, i)
        end
        return sum
    "#).call(buf1.clone())?;

    let sum2: u32 = lua.load(r#"
        local b = ...
        local sum = 0
        for i=0, buffer.len(b)-1 do
            sum = sum + buffer.readu8(b, i)
        end
        return sum
    "#).call(buf2.clone())?;

    let expected_sum = b"hello, world".iter().map(|&b| b as u32).sum::<u32>();
    assert_eq!(sum1, expected_sum);
    assert_eq!(sum2, expected_sum);

    // GC both
    drop(buf1);
    drop(buf2);
    lua.gc_collect()?;

    Ok(())
}

#[test]
fn test_external_buffer_downcast() -> Result<()> {
    let lua = Lua::new();
    let data = b"hello, world".to_vec();
    let buf = lua.create_external_buffer(data)?;

    assert_eq!(buf.len(), 12);
    
    // Downcast to Vec<u8>
    let vec_ref = buf.downcast_ref::<Vec<u8>>();
    assert!(vec_ref.is_some());
    assert_eq!(vec_ref.unwrap(), b"hello, world");

    // Try downcasting to wrong type
    let wrong_ref = buf.downcast_ref::<Vec<u16>>();
    assert!(wrong_ref.is_none());

    // Try downcasting a normal (non-external) buffer
    let normal_buf = lua.create_buffer(b"hello")?;
    assert!(normal_buf.downcast_ref::<Vec<u8>>().is_none());

    #[cfg(feature = "bytes")]
    {
        let bytes_data = bytes::Bytes::from("hello, bytes");
        let bytes_buf = lua.create_external_buffer(bytes_data.clone())?;
        
        let bytes_ref = bytes_buf.downcast_ref::<bytes::Bytes>();
        assert!(bytes_ref.is_some());
        assert_eq!(bytes_ref.unwrap().as_ref(), b"hello, bytes");
    }

    Ok(())
}

#[test]
fn test_external_buffer_arc() -> Result<()> {
    use std::sync::Arc;
    let lua = Lua::new();
    let data = Arc::new(b"hello, Arc".to_vec());
    let buf = lua.create_external_buffer(data.clone())?;

    assert_eq!(buf.len(), 10);
    assert_eq!(buf.to_vec(), b"hello, Arc");

    let arc_ref = buf.downcast_ref::<Arc<Vec<u8>>>();
    assert!(arc_ref.is_some());
    assert!(Arc::ptr_eq(arc_ref.unwrap(), &data));

    // Ensure memory lifecycle works on GC
    assert_eq!(Arc::strong_count(&data), 2);
    drop(buf);
    lua.gc_collect()?;
    lua.gc_collect()?;
    assert_eq!(Arc::strong_count(&data), 1);

    Ok(())
}
