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

    static MDX: Lazy<Mdx> = Lazy::new(|| {
        parse("/Users/jeffsky/.yadict/mdicts/英汉大词典（第二版）陆谷孙.mdx")
            .expect("failed to read mdx")
    });

    #[test]
    fn test_keys() -> anyhow::Result<()> {
        init();
        for record in MDX.items() {
            let key = String::from_utf8_lossy(record.key());
            if key.starts_with("cat") {
                let v = record
                    .value()
                    .map(|it| String::from_utf8_lossy(it).to_string());
                info!("{} -- {}", key, v.unwrap());
            }
        }

        Ok(())
    }

    #[test]
    fn test_query() -> anyhow::Result<()> {
        init();

        info!("{}", &*MDX);

        let result = MDX.lookup("bird");

        assert!(!result.is_empty());

        for record in result {
            let key = unsafe { std::str::from_utf8_unchecked(record.key()) };
            let value = unsafe { record.value().map(|b| std::str::from_utf8_unchecked(b)) };

            info!("{}: {:?}", key, value);
        }

        Ok(())
    }
}
