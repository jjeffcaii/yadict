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

        let dict = parse("/Users/jeffsky/.yadict/mdicts/英汉大词典（第二版）陆谷孙.mdx")?;

        let result = dict.get("bird");

        assert!(result.is_some());

        let record = result.unwrap();

        let key = unsafe { std::str::from_utf8_unchecked(record.key()) };
        let value = unsafe { record.value().map(|b| std::str::from_utf8_unchecked(b)) };

        info!("{}: {:?}", key, value);

        Ok(())
    }
}
