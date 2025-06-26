fn main() {
    loop {
        println!("I'm a dummy in a loop");
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
