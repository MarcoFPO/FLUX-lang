use std::io::Read;
use flux_ftl::parser::parse_ftl;

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let result = parse_ftl(&input);
    println!("{}", serde_json::to_string(&result).unwrap());
}
