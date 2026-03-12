use sled::Config;

fn main() {
    let db = Config::new().temporary(true).flush_every_ms(None).open().unwrap();
    db.insert(1u64.to_be_bytes(), b"hello").unwrap();
    if let Ok(Some((_, v))) = db.pop_min() {
        println!("Popped: {:?}", v.as_ref());
    } else {
        println!("Popped NONE!");
    }
}
