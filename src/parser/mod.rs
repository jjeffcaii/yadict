pub mod mdict;
pub mod parser;

pub use mdict::{Mdx, Record};
pub use parser::parse;

#[cfg(test)]
mod tests {
    use super::*;
    use once_cell::sync::Lazy;

    fn init() {
        pretty_env_logger::try_init_timed().ok();
    }

    static MDX: Lazy<Mdx> = Lazy::new(|| parse("/tmp/example.mdx").expect("failed to read mdx"));

    #[ignore]
    #[test]
    fn test_list() {
        init();
        info!("encoding: {}", &MDX.encoding);

        for key in MDX.keys() {
            info!("{:?}", crate::lang::decode(&MDX.encoding, key.text));
        }
    }

    #[test]
    fn test_query() -> anyhow::Result<()> {
        init();

        for word in &["cat", "dog", "猫", "狗"] {
            let result = MDX.lookup(word);

            assert!(!result.is_empty());

            for record in result {
                let key = record.key();
                let value = record.value();
                info!("{}: {:?}", key, value);
            }
        }

        Ok(())
    }
}
