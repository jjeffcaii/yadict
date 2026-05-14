pub mod mdict;
pub mod parser;

pub use mdict::{Mdx, Record};
pub use parser::parse;

#[cfg(test)]
mod tests {
    use super::*;

    fn init() {
        pretty_env_logger::try_init_timed().ok();
    }

    #[test]
    fn test_query() -> anyhow::Result<()> {
        init();

        let dict = parse("/Users/jeffsky/Downloads/col.mdx")?;
        // let dict = parse("/Users/jeffsky/Downloads/简明汉英词典.mdx")?;

        let result = dict.get("apple");

        assert!(result.is_some());

        let record = result.unwrap();

        let key = unsafe { std::str::from_utf8_unchecked(record.key()) };
        let value = unsafe { record.value().map(|b| std::str::from_utf8_unchecked(b)) };

        info!("{}: {:?}", key, value);

        Ok(())
    }
}
