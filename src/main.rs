use clap::{Parser, Subcommand};
use yadict::parser;

#[derive(Parser)]
#[command(name = "yadict", about = "MDict dictionary lookup tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Translate a word using a .mdx dictionary file
    Translate {
        /// Path to the .mdx dictionary file
        #[arg(short = 'd', long)]
        dict: String,

        /// Word to look up
        word: String,
    },
}

fn main() -> anyhow::Result<()> {
    env_logger::try_init().ok();

    let cli = Cli::parse();

    match cli.command {
        Commands::Translate { dict, word } => {
            let mdx = parser::parse(&dict)?;
            match mdx.get(&word) {
                Some(record) => {
                    let key = String::from_utf8_lossy(record.key());
                    match record.value() {
                        Some(bytes) => {
                            let value = String::from_utf8_lossy(bytes);
                            println!("{}: {}", key, value);
                        }
                        None => println!("{}: (no definition found)", key),
                    }
                }
                None => println!("'{}' not found in dictionary", word),
            }
        }
    }

    Ok(())
}
