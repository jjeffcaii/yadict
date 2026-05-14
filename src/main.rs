#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate log;

use anyhow::Result;
use yadict::parser;

fn main() {}

fn query(search: &str) -> Result<()> {
    let raw = open("/Users/jeffsky/Downloads/col.mdx")?;
    let dict = parser::parse(&raw);

    info!("begin====query");

    for item in dict.items() {
        if item.key == "apple" {
            info!("{:?}: {}", item.key, item.definition);
            break;
        }
    }

    Ok(())
}

fn open(path: &str) -> Result<Vec<u8>> {
    use std::fs::File;
    use std::io::Read;
    let mut f = File::open(path)?;
    let mut data = vec![];
    f.read_to_end(&mut data)?;

    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        pretty_env_logger::try_init_timed().ok();
    }

    #[test]
    fn test_query() {
        init();
        assert!(query("").is_ok())
    }
}
