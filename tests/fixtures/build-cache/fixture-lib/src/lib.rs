pub fn shared_number() -> String {
    let mut buffer = itoa::Buffer::new();
    buffer.format(42).to_owned()
}
