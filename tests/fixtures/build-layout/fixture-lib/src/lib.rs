pub fn shared_number() -> String {
    let mut buffer = itoa::Buffer::new();
    let number = if cfg!(feature = "wide") { 84 } else { 42 };
    buffer.format(number).to_owned()
}
