use crawli_lib::path_utils::parse_size;

fn main() {
    let sizes = ["0", "0 B", "0.0 MB", "0 KB", "0.00 B", "-", "1.5 M", "912.0 KiB", "1024"];
    for size in sizes {
        println!("{:?} -> {:?}", size, parse_size(size));
    }
}
