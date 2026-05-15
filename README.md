# yadict

A fast command-line MDict (`.mdx`) dictionary lookup tool with terminal color rendering.

## Installation

```bash
cargo install --path .
```

## Usage

### Look up a word

```bash
yadict translate <word>
```

Queries all `.mdx` dictionaries installed in `~/.yadict/mdicts/` and prints results separated by dividers. Output is rendered with ANSI colors.

```
$ yadict translate cat
cat /kæt/ CET4 TEM4 ( cats )
› 1
  N-COUNT A cat is a furry animal that has a long tail and sharp claws. Cats are
  often kept as pets.
...
```

### Install a dictionary

```bash
yadict add <url>
```

Downloads a remote `.mdx` file to `~/.yadict/mdicts/` and records it in a local cache. If the URL has already been downloaded and the file still exists, it is reused without re-downloading.

```
$ yadict add https://example.com/oxford.mdx
Downloading https://example.com/oxford.mdx ...
Saved to /Users/you/.yadict/mdicts/oxford.mdx
```

## Data directory

By default, yadict stores dictionaries and cache under `~/.yadict/`. Set the `YADICT_HOME` environment variable to use a different location:

```bash
export YADICT_HOME=/data/yadict
yadict translate hello
```

## Directory layout

```
$YADICT_HOME/
├── mdicts/          # installed .mdx dictionary files
│   ├── oxford.mdx
│   └── collins.mdx
└── cache.tsv        # URL → local path index (managed automatically)
```
