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
